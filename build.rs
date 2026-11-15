// Emits `gtk` where Cargo.toml's target table pulls in the gtk crate. That table is the one list
// of GTK platforms: Cargo resolves it before any build script runs, so the cfg is read from it.
use cfg_expr::{Expression, Predicate, targets::get_builtin_target_by_triple};

fn main() {
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rustc-check-cfg=cfg(gtk)");

    let manifest: toml::Table = std::fs::read_to_string("Cargo.toml")
        .expect("reading Cargo.toml")
        .parse()
        .expect("parsing Cargo.toml");
    let gtk_cfg = manifest
        .get("target")
        .and_then(|targets| targets.as_table())
        .and_then(|targets| {
            targets
                .iter()
                .find(|(_, table)| table.get("dependencies").and_then(|deps| deps.get("gtk")).is_some())
        })
        .map(|(cfg, _)| cfg.as_str())
        .expect("Cargo.toml has no [target.'cfg(...)'.dependencies] table naming gtk");
    let gtk_cfg = Expression::parse(gtk_cfg).expect("parsing the gtk target table's cfg");

    let triple = std::env::var("TARGET").expect("cargo sets TARGET for a build script");
    let target = get_builtin_target_by_triple(&triple).unwrap_or_else(|| panic!("cfg-expr does not know the target {triple}"));
    if gtk_cfg.eval(|predicate| matches!(predicate, Predicate::Target(tp) if tp.matches(target))) {
        println!("cargo::rustc-cfg=gtk");
    }
}
