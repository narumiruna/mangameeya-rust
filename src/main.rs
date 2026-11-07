#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    if let Err(error) = mangameeya_rust::ui::run() {
        eprintln!("MangaMeeya Rust: {error:#}");
        std::process::exit(1);
    }
}
