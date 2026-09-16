use mesh_sim::{run, Scenario};
fn main() {
    if let Err(error) = execute() {
        eprintln!("mesh-sim: {error}");
        std::process::exit(1)
    }
}
fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 || args[0] != "--scenario" || args[2] != "--output" {
        return Err(
            "usage: mesh-sim --scenario scenarios/f1-recovery.json --output <new-directory>".into(),
        );
    }
    let metadata = std::fs::metadata(&args[1])?;
    if metadata.len() > 4096 {
        return Err("scenario exceeds 4096 bytes".into());
    }
    let scenario: Scenario = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let report = run(scenario, std::path::Path::new(&args[3]))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
