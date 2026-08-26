fn main() {
    let proto = "../../proto/conduit.proto";
    println!("cargo:rerun-if-changed={proto}");
    prost_build::compile_protos(&[proto], &["../../proto"]).expect("protoc failed");
    println!("cargo:rerun-if-changed=conduit-control.manifest");
}
