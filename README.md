# I) Usage
Install or release a package in the rootfs, including extra files or directories specified with `[[package.metadata.rootfs]]` in the manifest (`Cargo.toml`) from the root package itself or any of its dependencies.

Usage: `cargo rootfs install [OPTIONS]`<br/>
Install package in the rootfs, keeping debug symbols.


Usage: `cargo rootfs release [OPTIONS]`<br/>
Install package in the rootfs, stripping debug symbols.

## I.i) Options
```
  -d, --dest <DIRECTORY>      Rootfs directory (default: /)
  -s, --altsrc <DIRECTORY>    Use an an alternative sources for files to install.
      --target <TRIPLE>       Install for target triple
  -v, --verbose               Use verbose output
  -h, --help                  Print help
```

## I.ii) Target Selection
```
      --lib                   Install only this package's library
      --bins                  Install all binaries
      --bin [<NAME>]          Install only the specified binary
```

## I.iii) Feature Selection
```
  -F, --features <FEATURES>   Space or comma separated list of features to activate
      --all-features          Activate all available features
      --no-default-features   Do not activate the `default` feature
```

## I.iv) Manifest Options
```
      --manifest-path <PATH>  Path to Cargo.toml
      --lockfile-path <PATH>  Path to Cargo.lock (unstable)
      --locked                Assert that `Cargo.lock` will remain unchanged
      --offline               Run without accessing the network
      --frozen                Equivalent to specifying both --locked and --offline
```

## I.v) Environment variables
The following environment variables can be specifed:
- `CARGO_BUILD_TARGET`
- `STRIP`

# II) cargo rootfs metadata format
This tool allows you to define in a crate manifest (`Cargo.toml`) which files or directory should be installed with the package.

The tool will read the metadata of the root crate and all its dependencies.


## II.i) Install a configuration file in the rootfs
```
[[package.metadata.rootfs]]
source = "odl/greeter.odl"
destination = "/etc/amx/greeter/greeter.odl"
permissions = "0644"
```

Equivalent to:
```
install -D -m 0644 "odl/greeter.odl" "/etc/amx/greeter/greeter.odl"
```

## II.ii) Install an executable script in the rootfs
```
[[package.metadata.rootfs]]
source = "scripts/greeter_helper.sh"
destination = "/etc/amx/greeter/greeter_helper.sh"
permissions = "0755"
```

Equivalent to:
```
install -D -m 0755 "scripts/greeter_helper.sh" "/etc/amx/greeter/greeter_helper.sh"
```

## II.iii) Install an init script in the rootfs
```
[[package.metadata.rootfs]]
source = "scripts/greeter_init.sh"
destination = "/etc/init.d/greeter"
permissions = "0755"
init = { start = 90, stop = 90 }
```

Equivalent to:
```
install -D -m 0755 "scripts/greeter_init.sh" "/etc/init.d/greeter"
ln -s ../init.d/greeter /etc/rc1.d/S90greeter
ln -s ../init.d/greeter /etc/rc6.d/K90greeter
```

## II.iv) Make a symbolic link
```
[[package.metadata.rootfs]]
source = "../init.d/greeter"
destination = "/etc/reset/greeter"
symbolic = true
```

Equivalent to:
```
ln -s ../init.d/greeter /etc/reset/greeter
```

## II.v) Make a symbolic link to the root crate
Use this special option to build 'single binary' application embedding multiple sub-application (similarly to busybox).

If the 'greeter' crate is a depedency of the 'meta-app' application, then specifying:
```
[[package.metadata.rootfs]]
root_crate_symlink = true
```

Will ensure than the 'greeter' application is a simple symbolic link on 'meta-app'.

Equivalent to:
```
ln -s meta-app /usr/bin/greeter
```

This can be an advantage when building a rootfs for an embedded system where you are looking to save FLASH memory.
