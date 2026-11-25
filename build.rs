fn main() {
    for file in ["native/shim.m", "native/shim.h", "native/mirror.m", "native/mirror.h", "native/annotate.m"] {
        println!("cargo:rerun-if-changed={file}");
    }
    cc::Build::new()
        .file("native/shim.m")
        .file("native/mirror.m")
        .file("native/annotate.m")
        .flag("-fobjc-arc")
        .flag("-Wno-unused-parameter")
        .compile("agentphone_shim");
    for framework in ["Foundation", "CoreGraphics", "AppKit", "ApplicationServices", "ImageIO", "Vision", "CoreText"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
