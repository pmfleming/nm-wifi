fn main() {
    if let Err(err) = nm_wifi::run() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}
