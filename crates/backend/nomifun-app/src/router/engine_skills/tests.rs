use super::*;
use nomifun_agent_contracts::*;
use nomifun_plugin_platform::{ArtifactStoreLimits, NeverCancel};

const BODY: &[u8] = b"---\ndescription: Frozen guide\n---\nFollow the selected guide.";

struct Fixture {
    _root: tempfile::TempDir,
    artifacts: FsPluginArtifactStore,
    stored: StoredPluginArtifact,
    lock: ResolvedSkillLock,
    skill: MaterializedSkill,
}

impl Fixture {
    fn load(&self, mode: LoadMode) -> Result<SelectedSkills, AppError> {
        load(vec![(self.lock.clone(), self.skill.clone())], &self.artifacts, mode)
    }
}

fn fixture(body: &[u8], resource_path: &str, resource: &[u8]) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let artifacts = FsPluginArtifactStore::new(root.path(), ArtifactStoreLimits::default()).unwrap();
    let display = LocalizedMetadata {
        name: "Guide".into(), description: "Frozen guide".into(),
        localized_names: BTreeMap::new(), localized_descriptions: BTreeMap::new(),
    };
    let package = PackageRef { id: "test.skill.loader".into(), version: "1.0.0".into() };
    let definition = SkillDefinition {
        id: "test.skill.loader.guide".into(), version: "1.0.0".into(), package: package.clone(),
        display: display.clone(),
        body_ref: LogicalArtifactRef {
            artifact_id: "guide-body".into(), normalized_relative_path: "resources/SKILL.md".into(),
            digest: digest_bytes(body),
        },
        resources: vec![SkillResourceRef {
            kind: SkillResourceKind::Reference,
            artifact: LogicalArtifactRef {
                artifact_id: "guide-reference".into(), normalized_relative_path: resource_path.into(),
                digest: digest_bytes(resource),
            },
        }],
        requires_capabilities: Vec::new(),
        supported_surfaces: capability_surface_declarations(["desktop"], [CapabilityConsumer::Agent]),
    };
    let main = b"export async function activate() { return {}; }";
    let manifest = PluginPackageV1Manifest {
        schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::PluginPackageV1,
        build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        package: PackageManifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            package_id: package.id, package_version: package.version, display,
            package_dependencies: Vec::new(), requires_runtime_features: Vec::new(),
            config_schema: StrictJsonValue(serde_json::json!({"type": "object", "additionalProperties": false})),
            provides_services: Vec::new(), requires_services: Vec::new(),
            entrypoint: JavaScriptEntrypointMetadata {
                normalized_relative_path: "main.mjs".into(), module_digest: digest_bytes(main),
                host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            }.into(),
            contributions: PackageContributions { skills: vec![definition.clone()], ..Default::default() },
        },
        schemas: BTreeMap::new(),
        supported_targets: BTreeSet::from([RuntimeTarget::from("x86_64-pc-windows-msvc")]),
        minimum_node_major: MINIMUM_NODE_MAJOR,
        dependency_lock_digest: DigestHex::from("d".repeat(64)), credential_slots: Vec::new(),
    };
    let files = BTreeMap::from([
        ("main.mjs".into(), main.to_vec()),
        ("resources/SKILL.md".into(), body.to_vec()),
        (resource_path.into(), resource.to_vec()),
        ("manifest.json".into(), canonical_json_bytes(&ArtifactEnvelope::new(manifest).unwrap()).unwrap()),
    ]);
    let stored = artifacts.store().import_files(&files, &NeverCancel).unwrap().stored;
    let contract_digest = digest_payload(&definition).unwrap();
    let mount_id = PluginMountId::from("test-skill-loader-mount");
    let contribution_lock = ContributionLock {
        source_kind: ContributionSourceKind::PluginMount,
        source_identity: "test-skill-loader-mount".into(), mount_id: Some(mount_id.clone()),
        plugin_product_id: None, mcp_binding_id: None,
        contribution_id: "test.skill.loader.guide".into(), contract_digest: contract_digest.clone(),
    };
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::ManagedLocal,
        source_identity: "test.skill.loader".into(), source_digest: None,
    };
    let lock = ResolvedSkillLock {
        skill: SkillRef { id: definition.id.clone(), version: definition.version.clone() },
        body_digest: definition.body_ref.digest.clone(), required_capabilities: BTreeSet::new(),
        contribution_lock: contribution_lock.clone(), resolved_mount_id: mount_id.clone(),
        resolved_source: source.clone(), target_artifact_digest: stored.artifact.artifact_digest.clone(),
    };
    let skill = MaterializedSkill {
        definition, contribution_id: contribution_lock.contribution_id.clone(), contract_digest,
        contribution_lock, target_artifact_digest: lock.target_artifact_digest.clone(), mount_id, source,
    };
    Fixture { _root: root, artifacts, stored, lock, skill }
}

#[test]
fn command_discovery_skips_resource_decoding_and_context_limits() {
    for (path, bytes, expected_error) in [
        ("resources/reference.txt", vec![0xff], "not UTF-8"),
        ("resources/reference.txt", vec![b'x'; 256 * 1024 + 1], "byte bound"),
        ("resources/image.png", b"not a PNG image".to_vec(), "unknown format"),
    ] {
        let fixture = fixture(BODY, path, &bytes);
        let commands = fixture.load(LoadMode::Commands).unwrap();
        assert_eq!(commands.commands.len(), 1);
        assert_eq!(commands.commands[0].lock, fixture.lock);
        assert_eq!(commands.commands[0].markdown.as_bytes(), BODY);
        assert!(commands.resources().is_empty());
        commands.validate_extra(&serde_json::json!({"skills": [fixture.lock.skill.id]})).unwrap();
        assert!(commands.validate_ids(&["unselected.guide".into()]).is_err());
        let error = fixture.load(LoadMode::Resources).err().expect("full loading must validate resources");
        assert!(error.to_string().contains(expected_error), "{path}: {error}");
    }
}

#[test]
fn full_loading_preserves_commands_and_typed_text_resources() {
    let fixture = fixture(BODY, "resources/reference.txt", b"Exact frozen reference");
    let commands = fixture.load(LoadMode::Commands).unwrap();
    let full = fixture.load(LoadMode::Resources).unwrap();
    assert_eq!(commands.ids(), full.ids());
    assert_eq!(commands.instructions(), full.instructions());
    assert_eq!(commands.commands[0].markdown, full.commands[0].markdown);
    assert_eq!(commands.commands[0].description, full.commands[0].description);
    assert_eq!(full.resources().len(), 1);
    let resource = full.resources().values().next().unwrap();
    assert!(matches!(&resource.content, EngineContextContent::Text { text } if text == "Exact frozen reference"));
    assert!(resource.provenance.contains(fixture.lock.target_artifact_digest.as_ref()));
}

#[test]
fn command_discovery_still_rejects_tampered_or_missing_artifact_files() {
    for path in ["resources/SKILL.md", "resources/reference.txt"] {
        for missing in [false, true] {
            let fixture = fixture(BODY, "resources/reference.txt", b"Exact frozen reference");
            let path = fixture.stored.package_root.join(path);
            if missing { std::fs::remove_file(path).unwrap(); }
            else { std::fs::write(path, b"tampered").unwrap(); }
            for mode in [LoadMode::Commands, LoadMode::Resources] {
                assert!(fixture.load(mode).is_err(), "artifact integrity is mandatory in both modes");
            }
        }
    }
}

#[test]
fn command_discovery_keeps_body_bounds_and_definition_checks() {
    for body in [vec![0xff], vec![b'x'; 16 * 1024 + 1]] {
        let fixture = fixture(&body, "resources/reference.txt", b"reference");
        for mode in [LoadMode::Commands, LoadMode::Resources] {
            assert!(fixture.load(mode).is_err());
        }
    }
    let mut fixture = fixture(BODY, "resources/reference.txt", b"reference");
    fixture.skill.definition.display.description = "changed".into();
    for mode in [LoadMode::Commands, LoadMode::Resources] {
        assert!(fixture.load(mode).err().unwrap().to_string().contains("compiled contribution"));
    }
}

#[test]
fn discovery_and_execution_require_the_exact_frozen_skill_source() {
    let fixture = fixture(BODY, "resources/reference.txt", b"reference");
    validate_lock(&fixture.lock, &fixture.skill).unwrap();
    for field in ["id", "version", "body", "mount", "source", "artifact", "contract", "dependency"] {
        let mut lock = fixture.lock.clone();
        match field {
            "id" => lock.skill.id = "another.guide".into(),
            "version" => lock.skill.version = "2.0.0".into(),
            "body" => lock.body_digest = digest_bytes(b"changed body"),
            "mount" => lock.resolved_mount_id = "another-mount".into(),
            "source" => lock.resolved_source.source_identity = "another-source".into(),
            "artifact" => lock.target_artifact_digest = digest_bytes(b"changed artifact"),
            "contract" => lock.contribution_lock.contract_digest = digest_bytes(b"changed contract"),
            "dependency" => { lock.required_capabilities.insert("another.capability".into()); }
            _ => unreachable!(),
        }
        assert!(validate_lock(&lock, &fixture.skill).is_err(), "{field}");
    }
}
