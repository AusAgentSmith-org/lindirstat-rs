use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(embed_scanner)");

    let assets = Path::new("assets");
    let x86 = assets.join("scanner-x86_64");
    let arm = assets.join("scanner-aarch64");

    if x86.exists() && arm.exists() {
        println!("cargo:rustc-cfg=embed_scanner");
    }

    println!("cargo:rerun-if-changed=assets/scanner-x86_64");
    println!("cargo:rerun-if-changed=assets/scanner-aarch64");
}
