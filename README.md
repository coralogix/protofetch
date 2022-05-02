# Protofetch
![CI](https://github.com/coralogix/protofetch/workflows/CI/badge.svg)
[![Apache 2 License License](http://img.shields.io/badge/license-APACHE2-blue.svg)](http://www.apache.org/licenses/LICENSE-2.0)
[![Crates.io](https://img.shields.io/crates/v/protofetch.svg)](https://crates.io/crates/protofetch)
![GitHub Stars](https://img.shields.io/github/stars/coralogix/protofetch.svg)

A source dependency management tool for Protobuf files.

## Motivation

---

At Coralogix we use protobuf extensively as a data format to communicate between services and also with the outside world (public APIS).

Without a dependency management tool, we would have to manually download and copy all the dependencies for each service, moreover, we would lack any ability to depend on a specific version.
This quickly not only becomes cumbersome but also prone to errors and painfully to deal with.

We need something better. Something that automates this work and makes it predictable. This is what Protofetch is for.

### Why Protofetch?

Protofetch aims to tackle the complexity of handling protobuf dependencies in a declarative fashion. 
It makes it trivial to declare dependencies and to manage them. 

## Roadmap

---

This project is still under development and is subject to changes in the future. 
We aim to achieve at least the following goals before releasing the first stable version.

- [x] Fetch dependencies based on git tag or branch
- [x] Cache dependencies locally by revision
- [x] Fetch transitive dependencies
- [ ] Declarative rules per dependency
  - [ ] Whitelisting
  - [ ] Blacklisting
  - [ ] Dependency pruning (remove ``proto`` files that are not needed)
- [ ] Prevent circular dependencies

## Getting Started

---

Protofetch is being released to cargo so to use it you can directly download the crate from the [crates.io](https://crates.io/crates/protofetch) 
and install it with `cargo install protofetch`. 

### Usage

```sh
   # -f forces lock file to be generated in every run
   protofetch fetch -f 
  ```

---

## Dependency management format

You will need to describe your dependencies in the `protofetch.toml` file (you can override the name but it is heavily discouraged.)

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

### Protofetch dependency toml example

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
