//! qserver binary: stdin commands in, protocol responses on stdout.
use std::io::BufRead;

fn main() {
    let stdin = std::io::stdin();
    let mut store = qserver::Store::new();
    for line in stdin.lock().lines() {
        let line = line.unwrap_or_default();
        let resp = store.handle(&line);
        println!("{}", resp);
        qserver::log_diagnostic(format!("handled {} bytes -> {}", line.len(), resp));
    }
}
