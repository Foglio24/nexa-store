fn main() {
    if std::env::args().any(|arg|arg == "--smoke-test") {
        match nexa_store_lib::backend::smoke_test() {
            Ok(result) => println!("{}",serde_json::to_string_pretty(&result).unwrap()),
            Err(error) => {
                eprintln!("{}",serde_json::json!({"ok":false,"error":error,"mutationsExecuted":false}));
                std::process::exit(1);
            }
        }
        return;
    }
    nexa_store_lib::run();
}
