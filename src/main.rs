fn main() {
    if let Err(err) = nm_api::run() {
        nm_api::report_error(&err);
        std::process::exit(1);
    }
}
