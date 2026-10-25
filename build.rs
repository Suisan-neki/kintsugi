use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=MUGIURUSHI_SOURCE_REVISION");
    let revision = env::var("MUGIURUSHI_SOURCE_REVISION")
        .ok()
        .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unrecorded".into());
    println!("cargo:rustc-env=SOURCE_REVISION={revision}");
}
