fn main() -> Result<(), Box<dyn std::error::Error>> {
    connectrpc_build::Config::new()
        .files(&["proto/sunbeam/memory/v1/memory.proto"])
        .includes(&["proto"])
        .include_file("_memory.rs")
        .compile()?;

    println!("cargo:rerun-if-changed=proto/sunbeam/memory/v1/memory.proto");
    Ok(())
}
