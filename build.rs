fn main() {
    println!("cargo:rerun-if-changed=assets");
    topcoat::tailwind::BuildConfig::new()
        .executable("tailwindcss")
        .input("styles.css")
        .render()
        .expect("failed to generate Tailwind CSS");
}
