use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::File,
    path::{Path, PathBuf},
};

use pyxis_parcel::{InodeAttr, Parcel};

use super::recipe::Recipe;
use crate::{get_home, get_parcel_path, ParcelProvider};

fn get_recipe_path(package: &str) -> PathBuf {
    let mut buf = get_home();
    buf.push(".pyxis/recipe/");
    buf.push(package);
    buf
}

fn load_recipe(package: &str) -> Recipe {
    let mut path = get_recipe_path(package);
    path.push("parcel.recipe");
    let file =
        File::open(path).unwrap_or_else(|_| panic!("Cannot find recipe file for {}", package));
    serde_yaml::from_reader(file).unwrap()
}

pub fn get_deps(package: &str) -> Vec<String> {
    let recipe = load_recipe(package);
    recipe.depends
}

pub fn get_version(package: &str) -> String {
    let recipe = load_recipe(package);
    recipe.version
}

pub fn parcel_build(package: &str) {
    let recipe = load_recipe(package);

    let mut parcel = Parcel::new();

    parcel.metadata.depends = recipe.depends;
    parcel.metadata.version = recipe.version;

    let time = std::time::SystemTime::now();
    let attr = InodeAttr {
        atime: time,
        ctime: time,
        mtime: time,
        uid:   0,
        gid:   0,
        nlink: 1,
        perm:  0o644,
        rdev:  0,
    };
    let pyxis_dir = parcel.add_directory(attr, BTreeMap::new());
    let provider_dir = parcel.add_directory(attr, BTreeMap::new());
    let parcel_dir = parcel.add_directory(attr, BTreeMap::new());
    parcel
        .insert_dirent(1, std::ffi::OsString::from(".PYXIS"), pyxis_dir)
        .unwrap();
    parcel
        .insert_dirent(pyxis_dir, std::ffi::OsString::from("local"), provider_dir)
        .unwrap();
    parcel
        .insert_dirent(provider_dir, std::ffi::OsString::from(package), parcel_dir)
        .unwrap();

    for (source, dest) in recipe.files {
        let mut path = get_recipe_path(package);
        path.push(source);

        let mut pathsofar = PathBuf::new();
        let mut parent = 0;
        for comp in PathBuf::from(dest.clone()).parent().unwrap().iter() {
            pathsofar.push(comp);
            if parcel.select(pathsofar.clone()) == None {
                let dir = parcel.add_directory(attr, BTreeMap::new());
                parcel.insert_dirent(parent, comp.to_owned(), dir).unwrap();
            }
            parent = parcel.select(pathsofar.clone()).unwrap();
        }
        let ino = parcel
            .add_file(
                pyxis_parcel::FileAdd::Name(path.as_os_str().to_owned()),
                attr,
                BTreeMap::new(),
            )
            .unwrap();
        parcel
            .insert_dirent(
                parent,
                OsString::from(PathBuf::from(dest).file_name().unwrap()),
                ino,
            )
            .unwrap();
    }

    if let Some(actions) = recipe.actions {
        let mut path = get_recipe_path(package);
        path.push(actions);
        let ino = parcel
            .add_file(
                pyxis_parcel::FileAdd::Name(path.as_os_str().to_owned()),
                attr,
                BTreeMap::new(),
            )
            .unwrap();
        parcel
            .insert_dirent(parcel_dir, OsString::from(".INSTALL"), ino)
            .unwrap();
    }

    let parcelpath = get_parcel_path(ParcelProvider::Local, package);
    std::fs::create_dir_all(parcelpath.parent().unwrap()).unwrap();
    let file = File::create(parcelpath).unwrap();
    parcel.store(file).unwrap();
}
