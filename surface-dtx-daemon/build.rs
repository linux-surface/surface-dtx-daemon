use std::env;
use clap_complete::shells;

include!("src/cli.rs");


fn main() {
    let outdir = env::var_os("CARGO_TARGET_DIR")
        .or_else(|| env::var_os("OUT_DIR"))
        .unwrap();

    let mut app = app();
    clap_complete::generate_to(shells::Bash, &mut app, "surface-dtx-daemon", &outdir).unwrap();
    clap_complete::generate_to(shells::Zsh,  &mut app, "surface-dtx-daemon", &outdir).unwrap();
    clap_complete::generate_to(shells::Fish, &mut app, "surface-dtx-daemon", &outdir).unwrap();
}
