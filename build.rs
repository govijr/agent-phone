fn main() {
    println!("cargo:rerun-if-changed=native/shim.m");
    println!("cargo:rerun-if-changed=native/shim.h");
    cc::Build::new()
        .file("native/shim.m")
        .flag("-fobjc-arc")
        .flag("-Wno-unused-parameter")
        .compile("agentphone_shim");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
}
