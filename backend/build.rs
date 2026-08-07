// SPDX-FileCopyrightText: 2026 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

/// Locate the shared proto tree.
///
/// It lives at the repository root so the backend and this crate compile the
/// same OTA contract. `cargo build` runs from the crate directory, while the
/// container build copies the tree to `/proto`.
fn shared_proto_root() -> PathBuf {
    for candidate in ["../proto", "/proto"] {
        let path = PathBuf::from(candidate);
        if path.join("ota/v1/ota.proto").exists() {
            return path;
        }
    }
    panic!("cannot find the shared proto tree in ../proto or /proto");
}

fn main() {
    // Compiled with rust-protobuf rather than prost: up-rust's UPayload
    // conversions require `protobuf::Message`. `pure()` avoids needing a protoc
    // binary in the builder image.
    let proto_root = shared_proto_root();
    protobuf_codegen::Codegen::new()
        .pure()
        .includes([&proto_root])
        .input(proto_root.join("ota/v1/ota.proto"))
        .cargo_out_dir("ota_proto")
        .run_from_script();

    println!("cargo:rerun-if-changed={}", proto_root.display());
}
