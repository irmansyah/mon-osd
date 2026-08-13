// build.rs
fn main() {
    println!("cargo:warning=build.rs is running");
    println!("cargo:rustc-link-search=framework=/System/Library/PrivateFrameworks");
}
