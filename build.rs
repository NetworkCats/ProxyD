use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("proxyd_descriptor.bin"))
        .compile_protos(&["proto/proxyd.proto"], &["proto"])?;

    Ok(())
}
