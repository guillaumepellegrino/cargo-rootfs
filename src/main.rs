use serde_json::value::Value;
use camino::Utf8Path as Path;
use camino::Utf8PathBuf as PathBuf;
use std::fs::Permissions;
use std::os::unix::fs::{PermissionsExt, symlink};
use serde::Deserialize;
use colored::Colorize;

#[derive(Default,Debug,Copy,Clone,PartialEq)]
enum Command {
    #[default]
    None,
    Install,
    Release,
    //Info,
}

#[derive(Default,Debug,Clone)]
pub struct CargoRootfsArgs {
    command: Command,

    // Options:
    dst: Option<PathBuf>,
    altsrc: Option<PathBuf>,
    target: Option<String>,
    all_bins_only: bool,
    bins_only: Vec<String>,
    lib_only: bool,
    verbose: u32,

    // Feature Selection:
    features: Vec<cargo_metadata::CargoOpt>,

    // Manifest Options:
    manifest_path: Option<PathBuf>,
    lockfile_path: Option<String>,
    locked: bool,
    offline: bool,
    frozen: bool,
}

#[derive(Debug,Clone,PartialEq)]
pub struct CargoRootfs {
    command: Command,
    dst: PathBuf,
    altsrc: Option<PathBuf>,
    metadata: cargo_metadata::Metadata,
    outdir: PathBuf,
}

#[derive(Debug,Clone,PartialEq, Deserialize)]
pub struct InitScript {
    start: Option<u32>,
    stop: Option<u32>,
}

#[derive(Debug,Clone,PartialEq, Deserialize)]
pub struct CargoRootfsRule {
    destination: Option<PathBuf>,
    source: Option<PathBuf>,
    permissions: Option<String>,
    symbolic: Option<bool>,
    root_crate_symlink: Option<bool>,
    init: Option<InitScript>,
}

fn strmode(mode: Option<u32>) -> String {
    if let Some(mode) = mode {
        format!("-m 0{mode:0o}")
    }
    else {
        String::new()
    }
}

fn recursive_copy(src: &Path, dst: &Path, mode: Option<u32>, depth: i32) {
    if depth > 20 {
        panic!("Recursive copy detected ({src:?})");
    }

    if src.is_file() {
        println!("install -D {} {:#?} {:#?}", strmode(mode), src, dst);
        let dstdir = dst.parent().unwrap();

        std::fs::create_dir_all(dstdir)
            .unwrap_or_else(|e| panic!("Failed to create directory {dstdir}: {e:?}"));

        std::fs::copy(src, dst)
            .unwrap_or_else(|e| panic!("Failed to copy {src} to {dst}: {e:?}"));

        if let Some(mode) = mode {
            let perms = Permissions::from_mode(mode);
            std::fs::set_permissions(dst, perms).unwrap();
        }
    }
    else if src.is_dir() {
        println!("install -d {} {:#?} {:#?}", strmode(mode), src, dst);
        std::fs::create_dir_all(dst).unwrap();
        if let Some(mode) = mode {
            let perms = Permissions::from_mode(mode);
            std::fs::set_permissions(dst, perms).unwrap();
        }
        for dir in src.read_dir_utf8().unwrap() {
            let dir = dir.unwrap();
            let name = dir.file_name();
            if name.starts_with(".") {
                continue;
            }
            let src = src.join(&name);
            let dst = dst.join(&name);
            recursive_copy(&src, &dst, mode, depth + 1);
        }
    }
    else {
        panic!("Artifact {src:?} not found")
    }
}

fn strip(file: &Path) {
    let program = std::env::var("STRIP")
        .unwrap_or("strip".into());
    println!("{} {}", program, file);

    std::process::Command::new(program)
        .arg(file)
        .output()
        .expect("strip error");
}

impl CargoRootfs {
    pub fn new(args: &CargoRootfsArgs) -> Self {
        let metadata = args.metadata();

        let mut outdir = PathBuf::from(&metadata.target_directory);
        if let Some(toolchain) = &args.target {
            outdir.push(toolchain);
        }
        else if let Ok(toolchain) = std::env::var("CARGO_BUILD_TARGET") {
            outdir.push(toolchain);
        }
        outdir.push("release");

        Self {
            command: args.command,
            dst: args.dst.clone().unwrap_or("/".into()),
            altsrc: args.altsrc.clone(),
            metadata,
            outdir,
        }
    }

    fn get_root_package(&self) -> &cargo_metadata::Package {
        let resolve = self.metadata.resolve.as_ref()
            .expect("Failed to resolve dependencies graph");
        let root = resolve.root.as_ref()
            .expect("No root package");
        self.get_package(root)
    }

    fn get_package(&self, id: &cargo_metadata::PackageId) -> &cargo_metadata::Package {
        for package in &self.metadata.packages {
            if package.id == *id {
                return package;
            }
        }
        panic!("Could not find {id}");
    }


    fn get_manifest_dir(&self, package: &cargo_metadata::Package) -> PathBuf {
        let manifest_dir = package.manifest_path.parent()
            .unwrap_or_else(|| panic!("[{}] Failed to get manifest directory", package.name));
        PathBuf::from(manifest_dir)
    }

    fn get_source_file(&self, package: &cargo_metadata::Package, source: &Path) -> PathBuf {
        if let Some(altsrc) = &self.altsrc {
            let altsrc = altsrc.join(&package.name).join(source);
            if altsrc.exists() {
                return altsrc;
            }
        }

        self.get_manifest_dir(package).join(source)
    }

    fn get_destination_file(&self, destination: &Path) -> PathBuf {
        // join() does not work on absolute path. We must strip the '/' character.
        let destination = destination.strip_prefix("/").unwrap_or(destination);
        self.dst.join(destination)
    }

    fn root_crate_symlink_bin(&self, package: &cargo_metadata::Package) {
        let root_package = self.get_root_package();
        if &root_package.name == &package.name {
            return;
        }

        let root_bin = root_package.targets.iter().find(
            |target| target.kind.contains(&cargo_metadata::TargetKind::Bin));
        let root_bin = match root_bin {
            Some(x) => x,
            None => return,
        };

        for target in &package.targets {
            if !target.kind.contains(&cargo_metadata::TargetKind::Bin) {
                continue;
            }

            let original = &root_bin.name;
            let link = self.dst.join("usr/bin").join(&target.name);

            println!("ln -sf {:#?} {:#?}", original, link);
            let _ = std::fs::remove_file(&link);
            return symlink(&original, &link).unwrap();
        }
    }

    fn interpret_metadata_rule(&self, package: &cargo_metadata::Package, i: usize, rule: &CargoRootfsRule) {
        if rule.root_crate_symlink == Some(true) {
            self.root_crate_symlink_bin(&package);
            return;
        }

        let rule_src = rule.source.as_ref()
            .unwrap_or_else(|| panic!("[{}] Missing package.metadata.rootfs.[{i}].src", package.name));
        let rule_dst = rule.destination.as_ref()
            .unwrap_or_else(|| panic!("[{}] Missing package.metadata.rootfs.[{i}].dst", package.name));
        let mode = rule.permissions.as_ref().map(|mode| {
            u32::from_str_radix(mode, 8)
                .unwrap_or_else(|_| panic!("[{}] package.metadata.rootfs.[{i}].mode is not an octal number", package.name))
        });

        if rule.symbolic == Some(true) {
            let original = rule_src;
            let link = self.get_destination_file(rule_dst);
            println!("ln -sf {:#?} {:#?}", original, link);
            if let Some(linkdir) = link.parent() {
                std::fs::create_dir_all(&linkdir).unwrap();
            }
            let _ = std::fs::remove_file(&link);
            return symlink(&original, &link).unwrap();
        }
        else {
            let src = self.get_source_file(&package, rule_src);
            let dst = self.get_destination_file(rule_dst);
            recursive_copy(&src, &dst, mode, 0);
        }

        if let Some(init) = &rule.init {
            let name = rule_dst.file_name().unwrap();
            let original = PathBuf::from("../init.d").join(&name);
            if let Some(order) = &init.start {
                let rcdir = self.dst.join("etc/rc1.d");
                let link = rcdir.join(format!("S{order}{name}"));
                println!("ln -sf {:#?} {:#?}", original, link);
                std::fs::create_dir_all(&rcdir).unwrap();
                let _ = std::fs::remove_file(&link);
                symlink(&original, &link).unwrap();
            }
            if let Some(order) = &init.stop {
                let rcdir = self.dst.join("etc/rc6.d");
                let link = rcdir.join(format!("K{order}{name}"));
                println!("ln -sf {:#?} {:#?}", original, link);
                std::fs::create_dir_all(&rcdir).unwrap();
                let _ = std::fs::remove_file(&link);
                symlink(&original, &link).unwrap();
            }
        }
    }

    fn install_dependency(&self, package: &cargo_metadata::Package) {
        if let Value::Array(dep_metadata) = &package.metadata["rootfs"] {
            let name = &package.name;
            for (i, rule) in dep_metadata.iter().enumerate() {
                let rule: CargoRootfsRule = serde_json::from_value(rule.clone())
                    .unwrap_or_else(|e| panic!("[{name}] Failed to parse package.metadata.rootfs.[{i}]: {e:?}"));
                self.interpret_metadata_rule(package, i, &rule);
            }
        }
    }

    pub fn install_dependencies(&self) {
        let resolve = self.metadata.resolve.as_ref()
            .expect("Failed to resolve dependencies graph");

        for node in &resolve.nodes {
            let package = self.get_package(&node.id);
            self.install_dependency(&package);
        }
    }

    pub fn install_bin(&self, filename: &str) {
        let src = self.outdir.join(filename);
        let dst = self.dst.join("usr/bin").join(filename);
        recursive_copy(&src, &dst, Some(0o0755), 0);

        if self.command == Command::Release {
            strip(&dst);
        }
    }

    pub fn install_bins(&self) {
        for package in self.metadata.workspace_packages() {
            for target in &package.targets {
                if target.kind.contains(&cargo_metadata::TargetKind::Bin) {
                    self.install_bin(&target.name);
                }
            }
        }
    }

    pub fn install_lib(&self, name: &str) {
        let filename = format!("lib{name}.so");
        let src = self.outdir.join(&filename);
        let dst = self.dst.join("usr/lib").join(&filename);
        recursive_copy(&src, &dst, Some(0o0755), 0);
    }

    pub fn install_libs(&self) {
        for package in self.metadata.workspace_packages() {
            for target in &package.targets {
                if target.kind.contains(&cargo_metadata::TargetKind::DyLib)
                    || target.kind.contains(&cargo_metadata::TargetKind::CDyLib)
                {
                    self.install_lib(&target.name);
                }
            }
        }
    }
}

pub fn printusage(cmd: &str) {
    println!("{} {}", "Usage:".green().bold(), cmd.cyan().bold());
}

pub fn printopt(opt: &str, comment: &str) {
    println!("  {:<30} {}", opt.cyan().bold(), comment);
}

pub fn help() {
    println!("Install or release a package in the rootfs, including extra files or directories specified with {} in the manifest ({}) from the root package itself or any of its dependencies.",
        "[[package.metadata.rootfs]]".cyan().bold(),
        "Cargo.toml".cyan().bold());
    println!("");
    printusage("cargo rootfs install [OPTIONS]");
    println!("Install package in the rootfs, keeping debug symbols.");
    println!("");
    printusage("cargo rootfs release [OPTIONS]");
    println!("Install package in the rootfs, keeping debug symbols.");
    println!("");
    println!("{}", "Options:".green().bold());
    printopt("-d, --dest <DIRECTORY>", "Rootfs directory (default: /)");
    printopt("-s, --altsrc <DIRECTORY>", "Use an an alternative sources for files to install.");
    printopt("    --target <TRIPLE>", "Install for target triple");
    printopt("-v, --verbose", "Use verbose output");
    printopt("-h, --help", "Print help");
    println!("");
    println!("{}", "Target Selection:".green().bold());
    printopt("    --lib", "Install only this package's library");
    printopt("    --bins", "Install all binaries");
    printopt("    --bin [<NAME>]", "Install only the specified binary");
    println!("");
    println!("{}", "Feature Selection:".green().bold());
    printopt("-F, --features <FEATURES>", "Space or comma separated list of features to activate");
    printopt("    --all-features", "Activate all available features");
    printopt("    --no-default-features", "Do not activate the `default` feature");
    println!("");
    println!("{}", "Manifest Options:".green().bold());
    printopt("    --manifest-path <PATH>", "Path to Cargo.toml");
    printopt("    --lockfile-path <PATH>", "Path to Cargo.lock (unstable)");
    printopt("    --locked", "Assert that `Cargo.lock` will remain unchanged");
    printopt("    --offline", "Run without accessing the network");
    printopt("    --frozen", "Equivalent to specifying both --locked and --offline");
}

impl CargoRootfsArgs {
    fn metadata(&self) -> cargo_metadata::Metadata {
        let mut cmd = cargo_metadata::MetadataCommand::new();
        let mut other_options = vec![];
        for feature in &self.features {
            cmd.features(feature.clone());
        }
        if let Some(path) = &self.manifest_path {
            cmd.manifest_path(path);
        }
        if let Some(path) = &self.lockfile_path {
            other_options.push("--lockfile-path".into());
            other_options.push(path.into());
        }
        if self.locked {
            other_options.push("--locked".into());
        }
        if self.offline {
            other_options.push("--offline".into());
        }
        if self.frozen {
            other_options.push("--frozen".into());
        }
        cmd.other_options(other_options);
        cmd.exec()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn parse(&mut self) {
        let mut args = std::env::args();

        // skip the process name
        args.next();

        // Parse the command name
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "rootfs" => {},
                "install" => {
                    self.command = Command::Install;
                    break;
                },
                "release" => {
                    self.command = Command::Release;
                    break;
                },
                "--help"|"-h" => return help(),
                other => panic!("Unknown argument {}", other),
            }
        }

        if self.command == Command::None {
            help();
            std::process::exit(1);
        }

        // Parse the arguments
        while let Some(arg) = args.next() {
            match arg.as_str() {
                // options
                "-d"|"--dest" => {
                    self.dst = Some(PathBuf::from(args.next().unwrap()));
                },
                "-s"|"--altsrc" => {
                    self.altsrc = Some(PathBuf::from(args.next().unwrap()));
                },
                "--target" => {
                    self.target = Some(args.next().unwrap());
                },
                "--help"|"-h" => help(),
                "--verbose"|"-v" => self.verbose += 1,

                // target selections:
                "--lib" => {
                    self.lib_only = true;
                },
                "--bins" => {
                    self.all_bins_only = true;
                },
                "--bin" => {
                    self.bins_only.push(args.next().unwrap());
                },

                // feature selection:
                "-F"|"--features" => {
                    let features = args.next()
                        .unwrap()
                        .split(",")
                        .map(|x| x.to_string())
                        .collect();
                    self.features.push(cargo_metadata::CargoOpt::SomeFeatures(features));
                },
                "--all-features" => {
                    self.features.push(cargo_metadata::CargoOpt::AllFeatures);
                },
                "--no-default-features" => {
                    self.features.push(cargo_metadata::CargoOpt::NoDefaultFeatures);
                    //self.no_default_features = true;
                },

                // manifest options:
                "--manifest-path" => {
                    self.manifest_path = Some(PathBuf::from(args.next().unwrap()));
                },
                "--lockfile-path" => {
                    self.lockfile_path = Some(args.next().unwrap());
                },
                "--locked" => {
                    self.locked = true;
                },
                "--offline" => {
                    self.offline = true;
                },
                "--frozen" => {
                    self.frozen = true;
                },

                other => panic!("Unknown argument {}", other),
            }
        }
    }
}

fn main() {
    let mut args = CargoRootfsArgs::default();
    args.parse();

    let cargo_rootfs = CargoRootfs::new(&args);

    if args.all_bins_only {
        cargo_rootfs.install_bins();
    }
    for bin in &args.bins_only {
        cargo_rootfs.install_bin(bin);
    }
    if args.lib_only {
        cargo_rootfs.install_libs();
    }

    // install all by default
    if !args.all_bins_only && args.bins_only.is_empty() && !args.lib_only {
        cargo_rootfs.install_bins();
        cargo_rootfs.install_libs();
    }

    cargo_rootfs.install_dependencies();
}
