fn main() {
    // `sqlx::migrate!` tracks existing migration files, but Cargo otherwise
    // cannot notice a newly added file in an already compiled directory.
    println!("cargo:rerun-if-changed=migrations");
}
