const VERSION: &str = env!("CARGO_PKG_VERSION");

use pyxis_manage::*;

#[macro_use]
extern crate clap;

fn main() {
    let matches = clap_app!(pyxis =>
        (version: VERSION)
        (about: "Does awesome things")
        (@subcommand parcel =>
            (@subcommand fetch =>
                (@arg INPUT: +required "Sets the package to fetch from Arch repositories")
            )
        )
        (@subcommand image =>
            (@subcommand build =>
                (@arg MANIFEST: +required "")
            )
        )
    )
    .get_matches();
    if let Some(matches) = matches.subcommand_matches("parcel") {
        if let Some(matches) = matches.subcommand_matches("fetch") {
            pyxis_parcel_fetch(matches.value_of("INPUT").unwrap())
        }
        if let Some(matches) = matches.subcommand_matches("local") {
            pyxis_parcel_local(matches.value_of("INPUT").unwrap())
        }
    }
    if let Some(matches) = matches.subcommand_matches("image") {
        if let Some(matches) = matches.subcommand_matches("build") {
            pyxis_image_build(matches.value_of("MANIFEST").unwrap())
        }
    }
}
