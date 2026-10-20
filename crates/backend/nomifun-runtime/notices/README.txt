NomiFun redistributed product components

This directory carries the exact license and notice texts obtained from the
approved upstream component versions. manifest.json records each source and
SHA-256. The Node.js LICENSE was extracted from its approved official archive
and includes its bundled third-party licenses. Bun LICENSE.md preserves the
upstream linked-library notices and source/relink instructions. OfficeCLI
LICENSE and NOTICE are preserved verbatim.

Product packaging includes this directory. The component supplier also
extracts these texts and component provenance beside each managed executable.
A component version change must update its notices in the same release.

The Cua Driver application sidecar uses the same packaged notices directory;
it is not a managed external-tool supplier. Its MIT LICENSE, upstream
THIRD_PARTY_NOTICES and adapted private-worker license are preserved verbatim.
The platform-specific Computer worker dependency inventories and notice bundles
are generated from its locked Cargo metadata, with each upstream source,
declared SPDX expression, author list and license-text SHA-256 recorded. Exact
public source links also identify MPL-2.0 source availability. These inventories
are source/license provenance, not compilation or native acceptance evidence.

To refresh a worker target after a lock change, save `cargo metadata --locked
--format-version 1 --filter-platform TARGET --manifest-path
apps/computer-worker/Cargo.toml` as JSON, then run
`node scripts/computer/generate-notices.mjs /ABS/metadata.json PLATFORM TARGET` from
that host's environment. PLATFORM is windows, linux or macos. The tool reads
cached crate license files and, when a published crate omits its workspace
license, retrieves it from the exact public .cargo_vcs_info commit. It never
builds, installs or runs the worker. Regenerate the corresponding manifest file
hashes when publishing the new notice bundle. macOS inventory covers the union
of the arm64 and Intel locked package sets (merge the two actual metadata
package sets by package ID, and pass both target triples); source provenance is not a macOS
compile, signature or TCC result.
