//! `scrap build --platform ios|ios-sim` and `scrap run --platform …`: a
//! game as an iPhone or iPad app (docs/ios.md).
//!
//! The game crate is compiled for the phone (`aarch64-apple-ios`) or the
//! Simulator (`aarch64-apple-ios-sim`) and laid out as `<Title>.app`:
//!
//! ```text
//! <out>/<Title>.app/
//!   <game>                 the executable
//!   data/                  what `scrap build` ships (the library cooked
//!                          for a phone: ASTC, zstd), found by data_file
//!   Info.plist             named, versioned, held as scrap.ron's build: says
//!   Assets.car, AppIcon*   the icon and the launch screen's colour (actool)
//!   embedded.mobileprovision, _CodeSignature/   on a phone
//! ```
//!
//! No Xcode project: the app is made the way Xcode makes one (actool,
//! codesign) by the same tools Xcode ships. On a phone it is signed for
//! development with an "Apple Development" identity from the keychain and
//! a provisioning profile Xcode already downloaded that covers the app's
//! identifier, the identity and (when it is named) the phone: an exact one
//! first, else the team's wildcard. Installing goes through `xcrun simctl`
//! on the Simulator and `xcrun devicectl` on a phone, by cable or over the
//! network once the phone is paired.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use scrap::Project;

use crate::build::{package_for, Platform, Profile};

/// The oldest iOS an app is built for.
pub const MINIMUM_IOS: &str = "16.0";

/// Where the app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// An iPhone or iPad (arm64).
    Device,
    /// The Simulator on an Apple-silicon Mac.
    Simulator,
}

impl Target {
    /// `ios` (a phone) or `ios-sim`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ios" | "iphone" | "iphoneos" => Some(Target::Device),
            "ios-sim" | "ios-simulator" | "simulator" | "iphonesimulator" => Some(Target::Simulator),
            _ => None,
        }
    }

    pub fn triple(self) -> &'static str {
        match self {
            Target::Device => "aarch64-apple-ios",
            Target::Simulator => "aarch64-apple-ios-sim",
        }
    }

    fn sdk(self) -> &'static str {
        match self {
            Target::Device => "iphoneos",
            Target::Simulator => "iphonesimulator",
        }
    }

    fn platform_name(self) -> &'static str {
        match self {
            Target::Device => "iPhoneOS",
            Target::Simulator => "iPhoneSimulator",
        }
    }
}

/// How a phone's build is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signing {
    /// The Simulator's: ad hoc (`codesign -s -`).
    AdHoc,
    /// Found on this Mac: an Apple Development identity and a profile
    /// covering the app (see the module). `device`: the phone the profile
    /// must list, when known.
    Automatic { identity: Option<String>, profile: Option<PathBuf>, device: Option<String> },
}

/// An app made.
#[derive(Debug, Clone)]
pub struct App {
    pub path: PathBuf,
    pub bundle_id: String,
    pub executable: String,
    /// How it was signed: the identity and profile's names, or "ad hoc".
    pub signed: String,
}

/// Build `project` as an app for `target` into `out` (`build/ios` or
/// `build/ios-sim` by default).
pub fn build(project: &Project, out: &Path, profile: Profile, target: Target, signing: &Signing) -> Result<App> {
    let stale: Vec<String> = scrap_import::sync(project)
        .into_iter()
        .filter_map(|r| r.result.err().map(|e| format!("{}: {e}", r.source.display())))
        .collect();
    for line in &stale {
        eprintln!("warning: {line}");
    }
    let manifest = project.root().join("Cargo.toml");
    if !manifest.is_file() {
        bail!("{} has no game crate to build", project.root().display());
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args(["build", "--message-format", "short", "--target", target.triple(), "--manifest-path"])
        .arg(&manifest)
        .args(profile.cargo_args())
        .env("IPHONEOS_DEPLOYMENT_TARGET", MINIMUM_IOS);
    let output = command.output().context("running cargo")?;
    if !output.status.success() {
        bail!("the game did not build for {}:\n{}", target.triple(), String::from_utf8_lossy(&output.stderr));
    }
    let name = crate_name(project)?;
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { project.root().join(p) })
        .unwrap_or_else(|| project.root().join("target"));
    let executable = target_dir.join(target.triple()).join(profile_folder(profile)).join(&name);
    anyhow::ensure!(executable.is_file(), "no executable at {}", executable.display());

    let settings = &project.manifest().build;
    let title = app_title(project);
    let app = out.join(format!("{title}.app"));
    if out.exists() {
        std::fs::remove_dir_all(out).with_context(|| format!("clearing {}", out.display()))?;
    }
    // The executable and data/ beside it, as `scrap build` lays them out,
    // the library cooked for a phone.
    package_for(project, &executable, &app, Platform::Mobile)?;
    let bundle_id = settings.bundle_id_for(project.name());
    let version = cargo_version(project).unwrap_or_else(|| "0.1.0".into());
    let build_number = build_number(project.root());
    std::fs::write(
        app.join("Info.plist"),
        info_plist(&InfoPlist {
            executable: &name,
            bundle_id: &bundle_id,
            title: &title,
            version: &version,
            build: &build_number,
            orientation: &settings.orientation,
            language: &project.manifest().game.language,
            target,
        }),
    )?;
    assets(project, &app, target, out)?;
    let signed = sign(&app, &bundle_id, target, signing, out)?;
    Ok(App { path: app, bundle_id, executable: name, signed })
}

fn profile_folder(profile: Profile) -> &'static str {
    match profile {
        Profile::Debug => "debug",
        Profile::Speed => "release",
        Profile::Size => "small",
    }
}

/// The app's name under its icon: the game's title, else the project's.
pub fn app_title(project: &Project) -> String {
    let title = project.manifest().game.title.trim();
    let title = if title.is_empty() { project.name() } else { title };
    title.replace('/', "-")
}

fn crate_name(project: &Project) -> Result<String> {
    cargo_field(project, "name").context("Cargo.toml has no package name")
}

fn cargo_version(project: &Project) -> Option<String> {
    cargo_field(project, "version")
}

/// The first `key = "…"` of the game's Cargo.toml (its `[package]`).
fn cargo_field(project: &Project, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(project.root().join("Cargo.toml")).ok()?;
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(key)?.trim().strip_prefix('=')?;
        Some(rest.trim().trim_matches('"').to_string())
    })
}

/// A number that grows with every build: the project's commits, else 1.
fn build_number(root: &Path) -> String {
    Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "1".into())
}

struct InfoPlist<'a> {
    executable: &'a str,
    bundle_id: &'a str,
    title: &'a str,
    version: &'a str,
    build: &'a str,
    orientation: &'a str,
    language: &'a str,
    target: Target,
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The app's Info.plist: what UIKit and the App's launch need. The scene
/// delegate is the engine's (scrap-shell `ios.rs`); the status bar hidden,
/// the screen whole, the orientations scrap.ron's.
fn info_plist(p: &InfoPlist) -> String {
    let orientations: &[&str] = match p.orientation {
        "landscape" => &["UIInterfaceOrientationLandscapeLeft", "UIInterfaceOrientationLandscapeRight"],
        "portrait" => &["UIInterfaceOrientationPortrait"],
        _ => &[
            "UIInterfaceOrientationPortrait",
            "UIInterfaceOrientationLandscapeLeft",
            "UIInterfaceOrientationLandscapeRight",
        ],
    };
    let list = |items: &[&str]| items.iter().map(|o| format!("<string>{o}</string>")).collect::<String>();
    let string = |k: &str, v: &str| format!("  <key>{k}</key><string>{}</string>\n", escape(v));
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n",
    );
    out += &string("CFBundleDevelopmentRegion", if p.language.is_empty() { "en" } else { p.language });
    out += &string("CFBundleExecutable", p.executable);
    out += &string("CFBundleIdentifier", p.bundle_id);
    out += &string("CFBundleInfoDictionaryVersion", "6.0");
    out += &string("CFBundleName", p.title);
    out += &string("CFBundleDisplayName", p.title);
    out += &string("CFBundlePackageType", "APPL");
    out += &string("CFBundleShortVersionString", p.version);
    out += &string("CFBundleVersion", p.build);
    out += &format!("  <key>CFBundleSupportedPlatforms</key><array><string>{}</string></array>\n", p.target.platform_name());
    out += &string("DTPlatformName", p.target.sdk());
    out += &string("MinimumOSVersion", MINIMUM_IOS);
    out += "  <key>LSRequiresIPhoneOS</key><true/>\n";
    out += "  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>\n";
    out += "  <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string><string>metal</string></array>\n";
    out += "  <key>UIRequiresFullScreen</key><true/>\n";
    out += "  <key>UIStatusBarHidden</key><true/>\n";
    out += "  <key>ITSAppUsesNonExemptEncryption</key><false/>\n";
    out += "  <key>UIApplicationSupportsIndirectInputEvents</key><true/>\n";
    out += "  <key>UILaunchScreen</key><dict><key>UIColorName</key><string>LaunchBackground</string></dict>\n";
    out += &format!("  <key>UISupportedInterfaceOrientations</key><array>{}</array>\n", list(orientations));
    out += &format!("  <key>UISupportedInterfaceOrientations~ipad</key><array>{}</array>\n", list(orientations));
    out += concat!(
        "  <key>UIApplicationSceneManifest</key>\n  <dict>\n",
        "    <key>UIApplicationSupportsMultipleScenes</key><false/>\n",
        "    <key>UISceneConfigurations</key>\n    <dict>\n",
        "      <key>UIWindowSceneSessionRoleApplication</key>\n      <array><dict>\n",
        "        <key>UISceneConfigurationName</key><string>Default</string>\n",
        "        <key>UISceneDelegateClassName</key><string>ScrapSceneDelegate</string>\n",
        "      </dict></array>\n    </dict>\n  </dict>\n",
    );
    out += "</dict>\n</plist>\n";
    out
}

/// `#rrggbb` as 0–1 components; black when it does not read.
fn colour(hex: &str) -> [f32; 3] {
    let h = hex.trim().trim_start_matches('#');
    let c = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0) as f32 / 255.0;
    if h.len() == 6 {
        [c(0), c(2), c(4)]
    } else {
        [0.0; 3]
    }
}

/// The icon and the launch screen's colour, compiled into the app by
/// actool (an asset catalog made for the purpose under `work`), and what
/// it says of them merged into the Info.plist.
fn assets(project: &Project, app: &Path, target: Target, work: &Path) -> Result<()> {
    let settings = &project.manifest().build;
    let catalog = work.join("work").join("Assets.xcassets");
    let _ = std::fs::remove_dir_all(&catalog);
    std::fs::create_dir_all(catalog.join("LaunchBackground.colorset"))?;
    std::fs::write(catalog.join("Contents.json"), r#"{"info":{"author":"scrap","version":1}}"#)?;
    let [r, g, b] = colour(&settings.background);
    std::fs::write(
        catalog.join("LaunchBackground.colorset/Contents.json"),
        format!(
            r#"{{"colors":[{{"idiom":"universal","color":{{"color-space":"srgb","components":{{"red":"{r:.3}","green":"{g:.3}","blue":"{b:.3}","alpha":"1.000"}}}}}}],"info":{{"author":"scrap","version":1}}}}"#
        ),
    )?;
    let icon = (!settings.icon.is_empty()).then(|| project.root().join(&settings.icon));
    if let Some(icon) = &icon {
        anyhow::ensure!(icon.is_file(), "scrap.ron's build icon {} is not there", icon.display());
        let set = catalog.join("AppIcon.appiconset");
        std::fs::create_dir_all(&set)?;
        std::fs::copy(icon, set.join("icon-1024.png"))?;
        std::fs::write(
            set.join("Contents.json"),
            r#"{"images":[{"filename":"icon-1024.png","idiom":"universal","platform":"ios","size":"1024x1024"}],"info":{"author":"scrap","version":1}}"#,
        )?;
    }
    let partial = work.join("work").join("assets.plist");
    let mut actool = Command::new("xcrun");
    actool
        .args(["actool", "--compile"])
        .arg(app)
        .args(["--platform", target.sdk(), "--minimum-deployment-target", MINIMUM_IOS])
        .args(["--target-device", "iphone", "--target-device", "ipad", "--output-format", "human-readable-text"])
        .arg("--output-partial-info-plist")
        .arg(&partial);
    if icon.is_some() {
        actool.args(["--app-icon", "AppIcon"]);
    }
    actool.arg(&catalog);
    let output = actool.output().context("running xcrun actool (Xcode's)")?;
    let said = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || said.contains("error:") {
        bail!("actool: {said}{}", String::from_utf8_lossy(&output.stderr));
    }
    let merged = Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Merge {}", partial.display()))
        .arg(app.join("Info.plist"))
        .output()?;
    anyhow::ensure!(merged.status.success(), "PlistBuddy: {}", String::from_utf8_lossy(&merged.stderr));
    Ok(())
}

/// A signing identity in the keychain: its SHA-1 and its name.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub hash: String,
    pub name: String,
}

/// The code-signing identities in the keychain that can sign (valid ones).
pub fn identities() -> Result<Vec<Identity>> {
    let output = Command::new("security").args(["find-identity", "-v", "-p", "codesigning"]).output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (_, rest) = line.split_once(") ")?;
            let (hash, name) = rest.split_once(' ')?;
            Some(Identity { hash: hash.to_string(), name: name.trim().trim_matches('"').to_string() })
        })
        .collect())
}

/// A provisioning profile on this Mac, as much of it as signing needs.
#[derive(Debug, Clone)]
pub struct ProvisioningProfile {
    pub path: PathBuf,
    pub name: String,
    /// `TEAM.com.example.app`, or `TEAM.*` for the team's wildcard.
    pub app_id: String,
    pub team: String,
    /// The certificates it trusts, as SHA-1s (upper-case hex).
    pub certificates: Vec<String>,
    /// The phones it lists (none: any, as an enterprise's).
    pub devices: Vec<String>,
    pub development: bool,
    pub expired: bool,
    /// Its entitlements, as XML.
    pub entitlements: String,
}

/// Where Xcode keeps the profiles it downloaded.
fn profile_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ]
}

/// One value of a plist, as `plutil -extract … raw` says it.
fn extract(plist: &Path, key: &str) -> Option<String> {
    let output = Command::new("plutil").args(["-extract", key, "raw", "-o", "-"]).arg(plist).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A profile's contents, decoded (it is a signed message).
pub fn read_profile(path: &Path, work: &Path) -> Option<ProvisioningProfile> {
    let decoded = Command::new("security").args(["cms", "-D", "-i"]).arg(path).output().ok()?;
    if !decoded.status.success() {
        return None;
    }
    std::fs::create_dir_all(work).ok()?;
    let plist = work.join("profile.plist");
    std::fs::write(&plist, &decoded.stdout).ok()?;
    let app_id = extract(&plist, "Entitlements.application-identifier")?;
    let team = extract(&plist, "Entitlements.com\\.apple\\.developer\\.team-identifier")
        .or_else(|| extract(&plist, "TeamIdentifier.0"))
        .unwrap_or_else(|| app_id.split('.').next().unwrap_or("").to_string());
    let mut certificates = Vec::new();
    for i in 0.. {
        let Some(b64) = extract(&plist, &format!("DeveloperCertificates.{i}")) else { break };
        if let Some(hash) = sha1_of_base64(&b64) {
            certificates.push(hash);
        }
    }
    let mut devices = Vec::new();
    for i in 0.. {
        let Some(d) = extract(&plist, &format!("ProvisionedDevices.{i}")) else { break };
        devices.push(d);
    }
    let development = extract(&plist, "Entitlements.get-task-allow").as_deref() == Some("true");
    let expired = extract(&plist, "ExpirationDate").is_some_and(|date| date.as_str() < now_iso().as_str());
    let entitlements = Command::new("plutil")
        .args(["-extract", "Entitlements", "xml1", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    Some(ProvisioningProfile {
        path: path.to_path_buf(),
        name: extract(&plist, "Name").unwrap_or_default(),
        app_id,
        team,
        certificates,
        devices,
        development,
        expired,
        entitlements,
    })
}

/// Now, as plutil writes a date (`2027-10-24T13:28:31Z`).
fn now_iso() -> String {
    let output = Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]).output();
    output.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// The SHA-1 of base64 bytes (a certificate's DER), upper-case hex.
fn sha1_of_base64(b64: &str) -> Option<String> {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "base64 -D | shasum -a 1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    {
        use std::io::Write;
        child.stdin.take()?.write_all(b64.as_bytes()).ok()?;
    }
    let output = child.wait_with_output().ok()?;
    let hex = String::from_utf8_lossy(&output.stdout);
    let hex = hex.split_whitespace().next()?.to_ascii_uppercase();
    (hex.len() == 40).then_some(hex)
}

/// How well a profile's app id covers `team.bundle_id`: exact beats a
/// wildcard, a longer wildcard a shorter one; `None` if it does not.
fn covers(app_id: &str, bundle_id: &str) -> Option<usize> {
    let (_, id) = app_id.split_once('.')?;
    if id == bundle_id {
        return Some(usize::MAX);
    }
    let prefix = id.strip_suffix('*')?;
    bundle_id.starts_with(prefix).then_some(prefix.len())
}

/// The identity and profile to sign `bundle_id` with for a phone.
pub fn choose(bundle_id: &str, identity: Option<&str>, profile: Option<&Path>, device: Option<&str>, work: &Path) -> Result<(Identity, ProvisioningProfile)> {
    let mut found = identities()?;
    found.retain(|i| i.name.starts_with("Apple Development") || i.name.starts_with("iPhone Developer"));
    if let Some(wanted) = identity {
        found.retain(|i| i.name == wanted || i.hash.eq_ignore_ascii_case(wanted) || i.name.contains(wanted));
    }
    if found.is_empty() {
        bail!(
            "no Apple Development signing identity in the keychain{}: sign in to Xcode with the team's Apple ID (Settings → Accounts) and let it make one",
            identity.map(|i| format!(" matching {i}")).unwrap_or_default()
        );
    }
    let profiles: Vec<ProvisioningProfile> = match profile {
        Some(path) => vec![read_profile(path, work).with_context(|| format!("{} is not a provisioning profile", path.display()))?],
        None => profile_dirs()
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "mobileprovision"))
            .filter_map(|p| read_profile(&p, work))
            .collect(),
    };
    let mut best: Option<(usize, Identity, ProvisioningProfile)> = None;
    for p in profiles {
        if p.expired || !p.development {
            continue;
        }
        if let Some(d) = device {
            if !p.devices.is_empty() && !p.devices.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                continue;
            }
        }
        let Some(score) = covers(&p.app_id, bundle_id) else { continue };
        let Some(id) = found.iter().find(|i| p.certificates.iter().any(|c| c.eq_ignore_ascii_case(&i.hash))) else { continue };
        if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
            best = Some((score, id.clone(), p));
        }
    }
    match best {
        Some((_, identity, profile)) => Ok((identity, profile)),
        None => bail!(
            "no development provisioning profile on this Mac covers {bundle_id}{} with {}: open Xcode, sign in with the team (Settings → Accounts) and build any app for that phone once with automatic signing — or pass --profile FILE",
            device.map(|d| format!(" on {d}")).unwrap_or_default(),
            found.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Sign the app: ad hoc for the Simulator; for a phone with an identity
/// and a profile (embedded), the entitlements the profile allows for this
/// app's identifier.
fn sign(app: &Path, bundle_id: &str, target: Target, signing: &Signing, out: &Path) -> Result<String> {
    let work = out.join("work");
    std::fs::create_dir_all(&work)?;
    let (hash, said, entitlements) = match (target, signing) {
        (Target::Simulator, _) | (_, Signing::AdHoc) => ("-".to_string(), "ad hoc".to_string(), None),
        (Target::Device, Signing::Automatic { identity, profile, device }) => {
            let (identity, profile) = choose(bundle_id, identity.as_deref(), profile.as_deref(), device.as_deref(), &work)?;
            std::fs::copy(&profile.path, app.join("embedded.mobileprovision"))?;
            let team = &profile.team;
            // The profile's entitlements, its wildcard made this app's.
            let wild = profile.app_id.clone();
            let mine = format!("{team}.{bundle_id}");
            let text = profile.entitlements.replace(&format!("<string>{wild}</string>"), &format!("<string>{mine}</string>"));
            let file = work.join("entitlements.plist");
            std::fs::write(&file, text)?;
            (identity.hash.clone(), format!("{} with \"{}\" (team {team})", identity.name, profile.name), Some(file))
        }
    };
    let mut codesign = Command::new("codesign");
    codesign.args(["--force", "--sign", &hash, "--timestamp=none"]);
    if let Some(file) = &entitlements {
        codesign.arg("--entitlements").arg(file).arg("--generate-entitlement-der");
    }
    let output = codesign.arg(app).output().context("running codesign")?;
    if !output.status.success() {
        bail!("codesign: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(said)
}

/// A Simulator by name or UDID (booted first), else the booted one, else
/// the first iPhone: its UDID, booted.
pub fn simulator(wanted: Option<&str>) -> Result<String> {
    let output = Command::new("xcrun").args(["simctl", "list", "devices", "available"]).output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // "    iPhone 17 (B35B…) (Booted) "
    let devices: Vec<(String, String, bool)> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let open = line.find(" (")?;
            let name = line[..open].to_string();
            let rest = &line[open + 2..];
            let close = rest.find(')')?;
            let udid = rest[..close].to_string();
            (udid.len() == 36).then(|| (name, udid, line.contains("(Booted)")))
        })
        .collect();
    let pick = match wanted {
        Some(w) => devices.iter().filter(|(n, u, _)| n == w || u.eq_ignore_ascii_case(w)).max_by_key(|d| d.2),
        None => devices.iter().find(|d| d.2).or_else(|| devices.iter().find(|d| d.0.starts_with("iPhone"))),
    };
    let Some((name, udid, booted)) = pick else {
        bail!("no Simulator {}: `xcrun simctl list devices available`", wanted.unwrap_or("for an iPhone"));
    };
    if !booted {
        eprintln!("booting the Simulator {name}");
        let _ = Command::new("xcrun").args(["simctl", "boot", udid]).status();
        let _ = Command::new("xcrun").args(["simctl", "bootstatus", udid, "-b"]).stdout(Stdio::null()).status();
    }
    Ok(udid.clone())
}

/// Install `app` on a Simulator and start it, `env` its environment;
/// `console`: wait for it, its output on this terminal.
pub fn run_on_simulator(app: &App, udid: &str, env: &[(String, String)], console: bool) -> Result<bool> {
    let ok = Command::new("xcrun").args(["simctl", "install", udid]).arg(&app.path).status()?.success();
    anyhow::ensure!(ok, "simctl could not install {}", app.path.display());
    let mut launch = Command::new("xcrun");
    launch.args(["simctl", "launch", "--terminate-running-process"]);
    if console {
        launch.arg("--console-pty");
    }
    launch.args([udid, &app.bundle_id]);
    for (k, v) in env {
        launch.env(format!("SIMCTL_CHILD_{k}"), v);
    }
    Ok(launch.status()?.success())
}

/// A paired phone by name or identifier, as devicectl knows it: its
/// identifier, and whether it can be reached now.
pub fn phone(wanted: Option<&str>, work: &Path) -> Result<(String, String, bool)> {
    std::fs::create_dir_all(work)?;
    let json = work.join("devices.json");
    let output = Command::new("xcrun")
        .args(["devicectl", "list", "devices", "--quiet", "--json-output"])
        .arg(&json)
        .output()?;
    anyhow::ensure!(output.status.success(), "devicectl: {}", String::from_utf8_lossy(&output.stderr));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&json)?)?;
    let devices = value["result"]["devices"].as_array().cloned().unwrap_or_default();
    let mut phones = Vec::new();
    for d in devices {
        let name = d["deviceProperties"]["name"].as_str().unwrap_or("").to_string();
        let id = d["identifier"].as_str().unwrap_or("").to_string();
        let udid = d["hardwareProperties"]["udid"].as_str().unwrap_or("").to_string();
        let platform = d["hardwareProperties"]["platform"].as_str().unwrap_or("");
        let state = d["connectionProperties"]["tunnelState"].as_str().unwrap_or("");
        let pairing = d["connectionProperties"]["pairingState"].as_str().unwrap_or("");
        if platform != "iOS" {
            continue;
        }
        let reachable = pairing == "paired" && state != "unavailable";
        if wanted.is_none_or(|w| w == name || w.eq_ignore_ascii_case(&udid) || w.eq_ignore_ascii_case(&id)) {
            phones.push((if udid.is_empty() { id } else { udid }, name, reachable));
        }
    }
    phones.sort_by_key(|p| !p.2);
    phones.into_iter().next().with_context(|| format!("no paired iPhone {}: `xcrun devicectl list devices`", wanted.unwrap_or("")))
}

/// Install `app` on a phone and start it, `env` its environment.
pub fn run_on_phone(app: &App, device: &str, env: &[(String, String)]) -> Result<bool> {
    let ok = Command::new("xcrun")
        .args(["devicectl", "device", "install", "app", "--device", device])
        .arg(&app.path)
        .status()?
        .success();
    if !ok {
        return Ok(false);
    }
    let mut launch = Command::new("xcrun");
    launch.args(["devicectl", "device", "process", "launch", "--device", device, "--terminate-existing"]);
    if !env.is_empty() {
        let map: serde_json::Map<String, serde_json::Value> = env.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
        launch.arg("--environment-variables").arg(serde_json::Value::Object(map).to_string());
    }
    launch.arg(&app.bundle_id);
    Ok(launch.status()?.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_covers_its_app_exactly_or_by_its_wildcard() {
        assert_eq!(covers("TEAM.com.a.game", "com.a.game"), Some(usize::MAX));
        assert_eq!(covers("TEAM.*", "com.a.game"), Some(0));
        assert_eq!(covers("TEAM.com.a.*", "com.a.game"), Some(6));
        assert_eq!(covers("TEAM.com.b.*", "com.a.game"), None);
        assert_eq!(covers("TEAM.com.a.other", "com.a.game"), None);
    }

    #[test]
    fn the_info_plist_says_the_app_and_its_scene_delegate() {
        let text = info_plist(&InfoPlist {
            executable: "game",
            bundle_id: "com.a.game",
            title: "A & B",
            version: "0.2.0",
            build: "17",
            orientation: "landscape",
            language: "ru",
            target: Target::Device,
        });
        assert!(text.contains("<key>CFBundleIdentifier</key><string>com.a.game</string>"));
        assert!(text.contains("<string>A &amp; B</string>"));
        assert!(text.contains("ScrapSceneDelegate"));
        assert!(text.contains("UIInterfaceOrientationLandscapeLeft") && !text.contains("UIInterfaceOrientationPortrait<"));
        assert!(text.contains("<string>iPhoneOS</string>"));
        assert_eq!(colour("#0b0a0d"), [11.0 / 255.0, 10.0 / 255.0, 13.0 / 255.0]);
    }
}
