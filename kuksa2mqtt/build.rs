fn main() {
    tonic_build::configure()
        .compile(
            &["proto/kuksa/val/v1/val.proto"],
            &["proto"],
        )
        .expect("failed to compile kuksa proto files");
}
