# conan-build
Extracts compiler flags from Conan packages for integration with Rust build scripts

Read zeromq-sys-sample/build.rs for an example on how to write a build script that
links with an external package from conan.

Remarks: It is recommended to not run `conan install` within build.rs.