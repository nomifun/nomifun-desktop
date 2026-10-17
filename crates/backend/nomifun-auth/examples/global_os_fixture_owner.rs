//! Prepare an isolated macOS native-acceptance owner through the real setup
//! router. This is fixture preparation, never native product acceptance.
//!
//! First use --prepare-empty-fixture /private/tmp/nomifun-global-os-UNIQUE0001/native-data,
//! then let the signed App bootstrap that empty directory through its real
//! dataset coordinator. Only then use --initialize-running-fixture with that
//! path. This tool never fabricates startup receipts or precreates a database.
//! Credentials are written once to separate 0600 credential.json, never printed.

#[cfg(target_os = "macos")]
mod fixture {
    use std::{
        error::Error,
        fs::{self, DirBuilder, OpenOptions},
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        path::{Component, Path, PathBuf},
        process::Command,
        sync::Arc,
    };

    use axum::{body::{Body, to_bytes}, http::{Request, StatusCode}};
    use nomifun_api_types::LoginResponse;
    use nomifun_auth::{
        AuthPolicy, AuthRouterState, CookieConfig, JwtService, QrTokenStore,
        auth_routes, generate_random_hex_secret,
    };
    use nomifun_db::{IUserRepository, SqliteUserRepository, init_database};
    use serde_json::json;
    use tower::ServiceExt;

    const MARKER: &str = "nomifun-global-os-native-acceptance-fixture.v1\n";

    fn exact_fixture_path(data: &Path) -> Result<PathBuf, Box<dyn Error>> {
        if !data.is_absolute() || data.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
            return Err("native-data must be an absolute path without dot components".into());
        }
        if data.file_name().and_then(|name| name.to_str()) != Some("native-data") {
            return Err("fixture data directory must be named native-data".into());
        }
        let root = data.parent().ok_or("fixture parent is missing")?;
        let name = root.file_name().and_then(|name| name.to_str()).ok_or("fixture parent name is invalid")?;
        let suffix = name.strip_prefix("nomifun-global-os-").ok_or("fixture parent must have the nomifun-global-os- prefix")?;
        if suffix.len() < 8 || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
            return Err("fixture parent must have a unique alphanumeric suffix of at least eight characters".into());
        }
        if root.parent() != Some(Path::new("/private/tmp")) {
            return Err("fixture parent must be a direct child of /private/tmp".into());
        }
        Ok(root.to_owned())
    }

    fn prepare_parent(root: &Path) -> Result<(), Box<dyn Error>> {
        match fs::symlink_metadata(root) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {},
            Ok(_) => return Err("fixture parent is not a real directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DirBuilder::new().mode(0o700).create(root)?,
            Err(error) => return Err(error.into()),
        }
        if fs::canonicalize(root)? != root {
            return Err("fixture parent must not contain symbolic-link aliases".into());
        }
        if fs::metadata(root)?.permissions().mode() & 0o077 != 0 {
            return Err("fixture parent must be private (0700); existing directory permissions are not modified".into());
        }
        let marker = root.join(".nomifun-global-os-fixture");
        match fs::symlink_metadata(&marker) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                if fs::read_to_string(&marker)? != MARKER { return Err("fixture marker does not match".into()); }
            },
            Ok(_) => return Err("fixture marker must be a regular file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::read_dir(root)?.next().is_some() {
                    return Err("unmarked fixture parent is not empty; refusing to adopt it".into());
                }
                let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(marker)?;
                file.write_all(MARKER.as_bytes())?;
                file.sync_all()?;
            },
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn running_app(data: &Path) -> Result<u32, Box<dyn Error>> {
        for path in [data.to_owned(), data.join("port.json"), data.join("nomifun-backend.db")] {
            let metadata=fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || (path==data && !metadata.is_dir()) || (path!=data && !metadata.is_file()) {
                return Err("running fixture paths must be real directories/files, never symlinks".into());
            }
        }
        if fs::metadata(data)?.permissions().mode() & 0o077 != 0 { return Err("native-data must remain private (0700)".into()); }
        let announcement:serde_json::Value=serde_json::from_slice(&fs::read(data.join("port.json"))?)?;
        let pid=announcement["pid"].as_u64().filter(|pid|*pid>1 && *pid<=u32::MAX as u64).ok_or("port.json has no valid App pid")? as u32;
        let port=announcement["port"].as_u64().filter(|port|*port>0 && *port<=65535).ok_or("port.json has no valid port")?;
        if announcement["host"]!="127.0.0.1" { return Err("fixture App must listen on IPv4 loopback".into()); }
        let command=Command::new("/bin/ps").args(["-ww","-p",&pid.to_string(),"-o","comm="]).output()?;
        let program=String::from_utf8(command.stdout)?.trim().to_owned();
        let app=program.strip_suffix("/Contents/MacOS/nomifun-desktop").ok_or("port.json pid is not a real NomiFun App executable")?;
        if !command.status.success() || !Path::new(app).is_absolute() { return Err("fixture App pid is not live".into()); }
        if !Command::new("/usr/bin/codesign").args(["--verify","--deep","--strict",app]).output()?.status.success() {
            return Err("fixture App signature verification failed".into());
        }
        let signing=Command::new("/usr/bin/codesign").args(["--display","--verbose=4",app]).output()?;
        let metadata=String::from_utf8(signing.stderr)?;
        if !signing.status.success() || !metadata.lines().any(|line|line=="Identifier=com.nomifun.desktop")
            || !metadata.lines().any(|line|line.starts_with("Authority=Developer ID Application:")) {
            return Err("fixture pid must belong to the signed com.nomifun.desktop App".into());
        }
        for args in [vec!["-nP".to_owned(),"-a".into(),"-p".into(),pid.to_string(),format!("-iTCP:{port}"),"-sTCP:LISTEN".into(),"-Fp".into()],
            vec!["-nP".into(),"-a".into(),"-p".into(),pid.to_string(),"-Fp".into(),"--".into(),data.join("nomifun-backend.db").to_string_lossy().into_owned()]] {
            let ownership=Command::new("/usr/sbin/lsof").args(args).output()?;
            if !ownership.status.success() || !String::from_utf8(ownership.stdout)?.lines().any(|line|line==format!("p{pid}")) {
                return Err("live App did not prove ownership of both the announced listener and exact fixture database".into());
            }
        }
        let status=Command::new("/usr/bin/curl").args(["--silent","--show-error","--fail","--max-time","5",&format!("http://127.0.0.1:{port}/api/auth/status")]).output()?;
        let status_body:serde_json::Value=serde_json::from_slice(&status.stdout)?;
        if !status.status.success() || status_body["success"]!=true || status_body["needs_setup"]!=true {
            return Err("live App does not have an uninitialized fixture owner".into());
        }
        Ok(pid)
    }

    pub async fn run() -> Result<(), Box<dyn Error>> {
        let mut args = std::env::args_os().skip(1);
        let mode=args.next().ok_or("expected --prepare-empty-fixture or --initialize-running-fixture")?;
        if mode!="--prepare-empty-fixture" && mode!="--initialize-running-fixture" { return Err("unsupported fixture mode".into()); }
        let data = PathBuf::from(args.next().ok_or("expected /private/tmp/nomifun-global-os-UNIQUE0001/native-data")?);
        if args.next().is_some() { return Err("expected exactly one native-data path".into()); }
        let root = exact_fixture_path(&data)?;
        prepare_parent(&root)?;
        if mode=="--prepare-empty-fixture" {
            if fs::symlink_metadata(&data).is_ok() { return Err("native-data already exists; use a new fixture root".into()); }
            DirBuilder::new().mode(0o700).create(&data)?;
            println!("{}",json!({"fixture_data":data,"bootstrap_required":true,"database_precreated":false}));
            return Ok(());
        }
        if mode!="--initialize-running-fixture" { return Err("unsupported fixture mode".into()); }
        let pid=running_app(&data)?;
        // Reserve the credential file before touching the dataset. All files
        // are new and private; failures remain isolated for investigation.
        let credential_path = root.join("credential.json");
        let mut credential = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&credential_path)?;
        // Existing finalized dataset only. The actual App coordinator, not
        // this auth fixture, owns all startup/storage-root receipts.
        let database = init_database(&data.join("nomifun-backend.db")).await?;
        let user_repo=Arc::new(SqliteUserRepository::new(database.pool().clone()));
        if user_repo.has_users().await? { return Err("fixture owner was already initialized".into()); }
        let username = "global-os-fixture";
        let password = generate_random_hex_secret();
        let router = auth_routes(AuthRouterState {
            auth_policy: AuthPolicy::Required,
            jwt_service: Arc::new(JwtService::new(generate_random_hex_secret())),
            user_repo,
            cookie_config: Arc::new(CookieConfig { secure: false, same_site: "Lax" }),
            qr_token_store: Arc::new(QrTokenStore::new()),
        });
        let body = serde_json::to_vec(&json!({"username": username, "password": password}))?;
        let response = router.oneshot(Request::builder().method("POST").uri("/api/auth/setup")
            .header("content-type", "application/json").body(Body::from(body))?).await?;
        if response.status() != StatusCode::OK {
            return Err(format!("production setup API returned {}; response body and credentials are not logged", response.status()).into());
        }
        let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
        let setup: LoginResponse = serde_json::from_slice(&bytes)?;
        if !setup.success || setup.user.username != username || setup.token.is_empty() {
            return Err("production setup API did not prove successful fixture-owner creation".into());
        }
        // Discard the setup JWT. The real signed App must issue its own JWT
        // via /login against the persisted normal password hash.
        drop(setup);
        credential.write_all(&serde_json::to_vec_pretty(&json!({"username": username, "password": password}))?)?;
        credential.write_all(b"\n")?;
        credential.sync_all()?;
        database.close().await;
        println!("{}", json!({"prepared": true, "fixture_data": data, "credential_file": credential_path,"verified_app_pid":pid,
            "scope": "production setup API fixture preparation; not native product acceptance"}));
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[tokio::main]
async fn main() {
    if let Err(error) = fixture::run().await {
        eprintln!("Global OS fixture owner preparation failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This isolated native-acceptance owner preparation requires macOS /private/tmp.");
    std::process::exit(1);
}
