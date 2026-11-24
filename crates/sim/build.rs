//! The target triple, for the libngspice that scripts/build-ngspice.sh builds into
//! target/ngspice/<triple>/ (found in development, see `shared::bundled_library`).
fn main() {
    println!("cargo:rustc-env=SIM_TARGET={}", std::env::var("TARGET").unwrap_or_default());
}
