## Protofetch

A source dependency management tool for Protobuf.

### Usage

```
Dependency management tool for Protocol Buffers files

USAGE:
    protofetch [OPTIONS] <SUBCOMMAND>

OPTIONS:
    -c, --cache-directory <CACHE_DIRECTORY>
            location of the protofetch cache directory [default: .protofetch_cache]

    -h, --help
            Print help information

    -l, --lockfile-location <LOCKFILE_LOCATION>
            location of the protofetch lock file [default: protofetch.lock]

    -m, --module-location <MODULE_LOCATION>
            location of the protofetch configuration toml [default: protofetch.toml]

    -V, --version
            Print version information

SUBCOMMANDS:
    clean      Cleans generated proto sources and lock file
    fetch      Fetches protodep dependencies defined in the toml configuration file
    help       Print this message or the help of the given subcommand(s)
    init       Creates an init protofetch setup in provided directory and name
    lock       Creates a lock file based on toml configuration file
    migrate    Migrates a protodep toml file to a protofetch format
```


### Module dependency toml format

```toml
name = "repository name"
description = "this is a repository"
proto_out_dir = "proto/src/dir/output"

[repo1]
  protocol = "https"
  url = "github.com/org/repo1"
  revision = "1.3.0"

[repo2]
  protocol = "ssh"
  url = "github.com/org/repo2"
  revision = "5.2.0"

[another-name]
protocol = "ssh"
url = "github.com/org/repo3"
revision = "a16f097eab6e64f2b711fd4b977e610791376223"
```
