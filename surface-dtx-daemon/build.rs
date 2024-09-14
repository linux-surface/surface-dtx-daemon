use std::env;
use std::path::PathBuf;
use clap_complete::shells;

include!("src/cli.rs");


fn main() {
    let outdir: PathBuf = env::var_os("CARGO_TARGET_DIR")
        .or_else(|| env::var_os("OUT_DIR"))
        .unwrap()
        .into();

    let rootdir = env::current_dir().unwrap();
    let rootdir = rootdir
        .parent().unwrap();

    println!("{rootdir:?}");

    // generate shell completions
    let mut app = app();
    clap_complete::generate_to(shells::Bash, &mut app, "surface-dtx-daemon", &outdir).unwrap();
    clap_complete::generate_to(shells::Zsh,  &mut app, "surface-dtx-daemon", &outdir).unwrap();
    clap_complete::generate_to(shells::Fish, &mut app, "surface-dtx-daemon", &outdir).unwrap();

    // copy config files
    let files = [
        "etc/dbus/org.surface.dtx.conf",
        "etc/dtx/attach.sh",
        "etc/dtx/detach.sh",
        "etc/dtx/surface-dtx-daemon.conf",
        "etc/systemd/surface-dtx-daemon.service",
        "etc/udev/40-surface_dtx.rules",
    ];

    for file in files {
        let src = rootdir.join(file);
        let tgt = outdir.join(file);

        std::fs::create_dir_all(tgt.parent().unwrap()).unwrap();
        std::fs::copy(src, tgt).unwrap();
    }
}
