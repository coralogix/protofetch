# Protofetch
![CI](https://github.com/coralogix/protofetch/workflows/CI/badge.svg)
[![Apache 2 License License](http://img.shields.io/badge/license-APACHE2-blue.svg)](http://www.apache.org/licenses/LICENSE-2.0)
[![Crates.io](https://img.shields.io/crates/v/protofetch.svg)](https://crates.io/crates/protofetch)
[![npm version](https://img.shields.io/npm/v/cx-protofetch.svg??style=flat)](https://www.npmjs.com/package/cx-protofetch)
![GitHub Stars](https://img.shields.io/github/stars/coralogix/protofetch.svg)

A source dependency management tool for Protobuf files.

## Motivation

If you use protobuf extensively as a data format for services to communicate with or to share your APIs with the outside world,
you need a way to get correct versions of protobuf files for each service and ability to depend on a specific version. 
This is needed on both server and client side. 
Without automation, it can quickly become cumbersome, error-prone and overall unmanageable.

To make it bearable, usable and stable, one needs tooling that automates this work and makes it predictable. This is what Protofetch aims to do.

## Why Protofetch?

Protofetch aims to tackle the complexity of handling protobuf dependencies in a declarative fashion.
It makes it trivial to declare dependencies and to manage them.

It gives you the ability to have:
* dependency on specific version/hash;
* predictable builds/test/CI that depend on protobufs;
* easy to read declarative specification of protobuf dependencies;
* automate fetching of the dependencies themselves with their transitive dependencies.
* caching of dependencies so that they can be shared across multiple projects.

## Roadmap

This project is still under development and is subject to change in the future.
We aim to achieve at least the following goals before releasing the first stable version.

- [x] Fetch dependencies based on git tag or branch
- [x] Cache dependencies locally by revision
- [x] Fetch transitive dependencies
- [x] Declarative rules per dependency
  - [x] Allow policies 
  - [x] Deny policies
  - [x] Dependency pruning (remove `proto` files that are not needed)
- [ ] Prevent circular dependencies

## Getting Started

You can download pre-built binaries from the [GitHub Releases](https://github.com/coralogix/protofetch/releases/latest) page.

Protofetch is also released to [crates.io](https://crates.io/crates/protofetch), so if you have a Rust toolchain installed, you can build Protofetch from source with `cargo install protofetch`.

### Usage

```sh
# Fetch proto sources, updating the lock file if needed.
protofetch fetch
   
# Verify the lock file, and fetch proto sources. Useful for CI.
protofetch fetch --locked
```

## Protofetch module

Each service using protofetch will require a module descriptor which uses `toml` format.
This descriptor is by default called `protofetch.toml` and is located in the root of the service's repository.
This can be changed, but it is heavily discouraged.

| Field        | Type         | Required  | Description                  |
|--------------|:-------------|:----------|:-----------------------------|
| name         | String       | Mandatory | A name of the defined module |
| description  | String       | Optional  | A description of the module  |
| dependencies | [Dependency] | Optional  | Dependencies to fetch        |

### Dependency format

| Field          | Type     | Required  | Description                                                                   | Example                                           |
|----------------|:---------|:----------|:------------------------------------------------------------------------------|:--------------------------------------------------|
| url            | String   | Mandatory | An address of the repository to checkout protobuf files from                  | "github.com/coralogix/cx-api-users/"              |
| revision       | String   | Optional  | A revision to checkout, this can either be a tagged version or a commit hash | v0.2                                              |
| branch         | Boolean  | Optional  | A branch to checkout, fetches last commit                                     | feature/v2                                        |
| protocol       | String   | Optional  | A protocol to use: [ssh, https]                                               | ssh                                               |
| allow_policies | [String] | Optional  | Allow policy rules                                                            | "/prefix/*", "*/subpath/*", "/path/to/file.proto" |
| deny_policies  | [String] | Optional  | Deny policy rules                                                             | "/prefix/*", "*/subpath/*", "/path/to/file.proto" |
| prune          | bool     | Optional  | Whether to prune unneeded transitive proto files                              | true /false                                       |
| transitive     | bool     | Optional  | Flags this dependency as transitive                                           | true /false                                       |
| content_roots  | [String] | Optional  | Which subdirectories to import from                                           | ["/myservice", "/com/org/client"]                                |

### Protofetch dependency toml example

```toml
name = "repository name"
description = "this is a repository"

[dep1]
url = "github.com/org/dep1"
protocol = "https"
revision = "1.3.0"
prune = true
allow_policies = ["/prefix/*", "*/subpath/*", "/path/to/file.proto"]

[dep2]
url = "github.com/org/dep2"
branch = "feature/v2"

[another-name]
url = "github.com/org/dep3"
revision = "a16f097eab6e64f2b711fd4b977e610791376223"
transitive = true

[scoped-down-dep4]
url = "github.com/org/dep4"
revision = "v1.1"
content_roots = ["/scope/path"]
allow_policies = ["prefix/subpath/scoped_path/*"]
```

## Git protocol

Protofetch supports accessing Git repositories using `ssh` or `https`. By default, Protofetch uses `ssh`. You can configure the default Git protocol with the `PROTOFETCH_GIT_PROTOCOL` environment variable.

It is also possible to set protocol in the `protofetch.toml`, but this should be only necessary if the Git server does not support both protocols. Otherwise, it is better to leave this field unset, to let users choose whichever protocol they prefer.

### SSH support

You need to have an SSH agent running, with your SSH key loaded:
```sh
ssh-add ~/.ssh/your-private-key
```

### HTTPS support

If you want to use https you need to configure git to use a [credentials helper](https://git-scm.com/docs/gitcredentials).

To support https when `2FA` is enabled you must generate a personal access token and set it as the password.
The following permissions are sufficient when creating the token.

![GitHub personal access token](readme-images/github-personal-access-token.png)

## Scope down multi API repo

In the case of a repo that supports multiple APIs, but only a specific directory is needed, a combination of `content_roots` and `allow_policies` can be used.

For example: the `dep4` repo contains the following:
```sh
  dep4
 ├──  scope
 │   ├──  path1
 │   └──  path2
 └──  scope2
     └──  unrelated
```
We only need protobuf files from `dep4/scope/path1`, where `path1` is the package name.

```toml
[scoped-down-dep4]
url = "github.com/org/dep4"
revision = "v1.1"
content_roots = ["/scope"]
allow_policies = ["path1/*"]
```


## Transitive dependency support and pruning

Protofetch supports pulling transitive dependencies for your convenience. 
However, there is some manual work involved if the dependencies do not define their own protofetch module.

In a situation where A depends on B, you should flag that dependency as transitive.

This is helpful especially when you take advantage of the pruning feature which allows you to only recursively fetch 
the proto files you actually need. With pruning enabled, protofetch will recursively find what protofiles your root 
protos depend on and fetch them for as long as they are imported (flag as transitive dependency or fetched from other modules).

Moreover, you can also use the allow_policies to scope down the root proto files you want from a dependency. 
As an example, the following module depends only on A's file `/proto/path/example.proto` but since pruning is enabled and 
B is flagged as transitive, if the allowed file has any file dependencies it will pull them and its dependencies, recursively.

IMPORTANT: If you are using the `prune` feature, you must also use the `transitive` feature. However, do not use transitive
unless you strictly want to pull the transitive dependencies. This is a workaround for dependencies that do not define
their protofetch file on their repo.

```toml
name = "repository name"
description = "this is a repository"
proto_out_dir = "proto/src/dir/output"

[A]
protocol = "https"
url = "github.com/org/A"
revision = "1.3.0"
allow_policies = ["/proto/path/example.proto"]
prune = true

[B]
protocol = "ssh"
url = "github.com/org/B"
revision = "5.2.0"
transitive = true
```
