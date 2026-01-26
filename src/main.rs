mod api;
mod config;
mod db;
mod ip;
mod metrics;
mod sync;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use std::sync::Arc;

use actix_web::{web, App, HttpServer};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use api::grpc::{configure_server, create_reflection_service, GrpcServerConfig, ProxyDService};
use api::rest::{configure, AppState};
use config::Config;
use db::Database;
use sync::scheduler::{initial_sync, run_scheduler};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("proxyd=info".parse()?))
        .init();

    info!("ProxyD starting...");

    let config = Config::default();

    std::fs::create_dir_all(&config.data_dir)?;

    let db = Database::open(&config.db_path())?;

    metrics::init_metrics();

    if let Err(e) = initial_sync(&db, &config).await {
        error!("Initial sync failed: {}", e);
        metrics::set_health_status(false);
    } else {
        metrics::set_health_status(true);
    }

    let db_for_rest = Arc::clone(&db);
    let db_for_grpc = Arc::clone(&db);
    let db_for_scheduler = Arc::clone(&db);
    let config_for_scheduler = config.clone();

    let shutdown_token = CancellationToken::new();
    let scheduler_token = shutdown_token.clone();

    let scheduler_handle = tokio::spawn(async move {
        run_scheduler(db_for_scheduler, config_for_scheduler, scheduler_token).await;
    });

    let grpc_addr = format!("0.0.0.0:{}", config.grpc_port).parse()?;
    let grpc_service = ProxyDService::new(db_for_grpc);

    let grpc_token = shutdown_token.clone();
    let grpc_config = GrpcServerConfig::default();
    let reflection_service = create_reflection_service();
    let grpc_handle = tokio::spawn(async move {
        info!("gRPC server listening on {}", grpc_addr);
        if let Err(e) = configure_server(&grpc_config)
            .add_service(reflection_service)
            .add_service(grpc_service.into_server())
            .serve_with_shutdown(grpc_addr, grpc_token.cancelled())
            .await
        {
            error!("gRPC server error: {}", e);
        }
        info!("gRPC server stopped");
    });

    let rest_addr = format!("0.0.0.0:{}", config.rest_port);
    info!("REST server listening on {}", rest_addr);

    let rest_server = HttpServer::new(move || {
        let state = AppState {
            db: Arc::clone(&db_for_rest),
        };
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure)
    })
    .workers(num_cpus::get())
    .bind(&rest_addr)?
    .run();

    let rest_handle = rest_server.handle();
    let rest_token = shutdown_token.clone();

    let rest_shutdown_task = tokio::spawn(async move {
        rest_token.cancelled().await;
        info!("REST server shutting down");
        rest_handle.stop(true).await;
    });

    let rest_server_task = tokio::spawn(async move {
        if let Err(e) = rest_server.await {
            error!("REST server error: {}", e);
        }
        info!("REST server stopped");
    });

    tokio::signal::ctrl_c().await?;
    info!("Received shutdown signal, initiating graceful shutdown");

    shutdown_token.cancel();

    let shutdown_timeout = std::time::Duration::from_secs(10);
    let _ = tokio::time::timeout(shutdown_timeout, async {
        let _ = tokio::join!(
            scheduler_handle,
            grpc_handle,
            rest_shutdown_task,
            rest_server_task,
        );
    })
    .await;

    info!("Shutdown complete");
    Ok(())
}
