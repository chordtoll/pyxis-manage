use clap::{crate_version, App, Arg};
use pyxis_manage::*;

fn main() {
    let matches = App::new("pyxis")
        .version(crate_version!())
        .author("chordtoll")
        .subcommand(
            App::new("parcel").subcommand(
                App::new("build").arg(
                    Arg::new("INPUT")
                        .required(true)
                        .help("The package to build. provider|package."),
                ),
            ),
        )
        .subcommand(
            App::new("image").subcommand(
                App::new("build").arg(
                    Arg::new("MANIFEST")
                        .required(true)
                        .help("The package manifest from which to build the image"),
                ),
            ),
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
