use mon_osd::{cache::Cache, ddc, ioav::AvService};

fn main() {
    // TODO: tray-icon setup — status item, menu with Get/Set shortcuts
    // TODO: on first launch, check whether already registered as a login
    //       item; if not, show a one-time "Enable at login?" menu entry
    //       that calls into SMAppService (via objc2 FFI) on click.
    // TODO: run loop stays alive — this replaces the CLI's exit-after-command model.
    println!("mon-osd-menubar starting (skeleton)");
}
