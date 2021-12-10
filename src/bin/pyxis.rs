const VERSION: &str = env!("CARGO_PKG_VERSION");

use pyxis_manage::*;

#[macro_use]
extern crate clap;

fn main() {
    let matches = clap_app!(pyxis =>
        (version: VERSION)
        (about: "The pyxis package manager. Quickly builds and deploys images for netboot.")
        (@subcommand parcel =>
            (@subcommand build =>
                (@arg INPUT: +required "The package to build. provider|package.")
            )
        )
        (@subcommand image =>
            (@subcommand build =>
                (@arg MANIFEST: +required "The package manifest from which to build the image")
            )
        )
    )
    .get_matches();
    if let Some(matches) = matches.subcommand_matches("parcel") {
        if let Some(matches) = matches.subcommand_matches("build") {
            pyxis_parcel_build_named(matches.value_of("INPUT").unwrap())
        }
    }
    if let Some(matches) = matches.subcommand_matches("image") {
        if let Some(matches) = matches.subcommand_matches("build") {
            pyxis_image_build(matches.value_of("MANIFEST").unwrap())
        }
    }
}
