use std::path::PathBuf;

mod chroot;
mod hookfile;
mod imagebuild;
mod providers;

pub use imagebuild::pyxis_image_build;

pub fn get_user() -> String {
    if let Ok(u) = std::env::var("SUDO_USER") {
        u
    } else {
        std::env::var("USER").unwrap()
    }
}

pub fn get_home() -> PathBuf {
    PathBuf::from(passwd::Passwd::from_name(&get_user()).unwrap().home_dir)
}

fn get_parcel_path(provider: ParcelProvider, package: &str) -> PathBuf {
    let mut buf = get_home();
    buf.push(".pyxis/parcel/");
    buf.push(provider.as_str());
    buf.push(format!("{}.parcel", package));
    buf
}

fn exists_parcel(provider: ParcelProvider, package: &str) -> bool {
    get_parcel_path(provider, package).exists()
}

#[derive(Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
enum ParcelProvider {
    Arch,
    Local,
}

impl ParcelProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            ParcelProvider::Arch => "arch",
            ParcelProvider::Local => "local",
        }
    }
}

fn get_provider(package: &str) -> (ParcelProvider, String) {
    if package.contains('|') {
        assert_eq!(package.matches('|').count(), 1);
        let mut package = package.split('|');
        let provider = package.next().unwrap();
        let package = package.next().unwrap();
        let provider = match provider {
            "arch" => ParcelProvider::Arch,
            "local" => ParcelProvider::Local,
            _ => unimplemented!(),
        };
        (provider, package.to_owned())
    } else {
        unimplemented!();
    }
}

pub fn pyxis_parcel_build_named(package: &str) {
    let (provider, package) = get_provider(package);
    pyxis_parcel_build(provider, &package);
}

fn pyxis_parcel_build(provider: ParcelProvider, package: &str) {
    match provider {
        ParcelProvider::Arch => providers::alpm::parcel_build(package),
        ParcelProvider::Local => providers::local::parcel_build(package),
    }
}

fn get_deps(provider: ParcelProvider, package: String) -> Vec<(ParcelProvider, String)> {
    match provider {
        ParcelProvider::Arch => providers::alpm::get_deps(&package)
            .iter()
            .map(|x| (provider, x.to_owned()))
            .collect(),
        ParcelProvider::Local => providers::local::get_deps(&package)
            .iter()
            .map(|x| get_provider(x))
            .collect(),
    }
}
