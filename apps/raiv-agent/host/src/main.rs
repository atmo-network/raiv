//! Native headless composition root; no host agent decisions or effect execution.
#[cfg(target_os = "linux")]
mod app;
fn main() {
    #[cfg(target_os = "linux")]
    match app::run(std::env::args_os().skip(1).take(14).collect()) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("raiv-agent: {error}");
            std::process::exit(2);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("raiv-agent headless disk application currently requires Linux");
        std::process::exit(2);
    }
}
