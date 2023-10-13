fn main() {
    tonic_build::configure()
        .type_attribute("routeguide.Point", "#[derive(Hash)]")
        .compile(&["routeguide.proto"], &["proto"])
        .unwrap();
}
