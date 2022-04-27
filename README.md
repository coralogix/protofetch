## Protofetch

A source dependency management tool for Protobuf.

### Usage

```
Dependency management tool for Protocol Buffers files

USAGE:
    protofetch [OPTIONS] <SUBCOMMAND>

OPTIONS:
    -c, --cache-directory <CACHE_DIRECTORY>
            location of the protofetch cache directory [default: cache]

    -h, --help
            Print help information

    -l, --lockfile-location <LOCKFILE_LOCATION>
            location of the protofetch lock file [default: protofetch.lock]

    -m, --module-location <MODULE_LOCATION>
            location of the protofetch configuration toml [default: protofetch.toml]

    -p, --proto-output-directory <PROTO_OUTPUT_DIRECTORY>
            name of the proto source files directory output, this will be used if config is not
            present in the toml config [default: proto_src]

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
## Dependency management

| Field         | Type      | Description                                                                |
|---------------|:----------|:---------------------------------------------------------------------------|
| name          | mandatory | the name of the defined module                                             |
| description   | mandatory | the description of the module                                              |  
| proto_out_dir | mandatory | the path to write the proto files to, relative to where the command is run |   


### Dependency configuration
| Field    | Type      |                                     Description                                     |                              Example |
|----------|:----------|:-----------------------------------------------------------------------------------:|-------------------------------------:|
| url      | mandatory |               the address of the repo to checkout protobuf files from               | "github.com/coralogix/cx-api-users/" |
| revision | mandatory | the revision to checkout from, this can either be a tagged version or a commit hash |                                 v0.2 |
| branch   | optional  |  branch can be used to override revision for testing purposes, fetches last commit  |                           feature/v2 |
| protocol | mandatory |                            protocol to use: [ssh, https]                            |                                  ssh |

### Module dependency toml format

```toml
name = "repository name"
description = "this is a repository"
proto_out_dir = "proto/src/dir/output"

[dep1]
  protocol = "https"
  url = "github.com/org/dep1"
  revision = "1.3.0"

[dep2]
  protocol = "ssh"
  url = "github.com/org/dep2"
  revision = "5.2.0"
  branch = "feature/v2"

[another-name]
protocol = "ssh"
url = "github.com/org/dep3"
revision = "a16f097eab6e64f2b711fd4b977e610791376223"
```
