//! Dedicated unprivileged worker process for one libkrun microVM.

fn main() {
    if let Err(error) = vm_libkrun::worker_main() {
        eprintln!("libkrun worker failed: {error}");
        std::process::exit(125);
    }
}
