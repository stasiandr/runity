//! Who makes the mesh: the seam, and the one implementation behind it.

use std::io::Read as _;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _, Result};
use base64::Engine as _;
use serde_json::{json, Value};

/// What a draft is made from.
#[derive(Debug, Clone)]
pub enum Input {
    /// Words: turned into a picture first, and the picture into a mesh.
    Text(String),
    /// A picture of the thing — a sketch, a concept, a photo — as the
    /// file's bytes and its media type.
    Image { bytes: Vec<u8>, mime: String },
}

/// What came back.
#[derive(Debug, Clone)]
pub struct Output {
    /// The mesh, as a binary glTF.
    pub glb: Vec<u8>,
    /// The picture the mesh was made from, when the provider drew one.
    pub picture: Option<Vec<u8>>,
    /// The networks, in the order they ran: `fal-ai/flux/schnell ->
    /// fal-ai/trellis`. Kept with the draft, so it says what made it.
    pub model: String,
    pub seed: Option<u64>,
}

/// A service that turns words or a picture into a mesh.
///
/// Blocking, and called off the editor's thread: generation takes tens of
/// seconds, and the session never waits for it. `progress` gets a line
/// whenever the stage changes, for the console and the job list.
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn generate(&self, input: &Input, progress: &mut dyn FnMut(String)) -> Result<Output>;
}

/// fal.ai: one key, the models as a queue of HTTP requests.
///
/// The default because it is the fastest way to a first mesh: nothing to
/// deploy, pay per call. Words go through FLUX schnell to a picture (about
/// a second), the picture through TRELLIS to a mesh (tens of seconds).
pub struct Fal {
    key: String,
    agent: ureq::Agent,
    pub image_model: String,
    pub mesh_model: String,
}

/// How a picture for image-to-3D should look: one thing, whole, on
/// nothing. A network reconstructing a shape from a scene invents the scene
/// too.
const PICTURE_STYLE: &str = "a single object, isolated, whole object in view, centered, \
three-quarter view from slightly above, plain white background, soft even light, \
stylized 3D game asset";

impl Fal {
    /// The key from `FAL_KEY`, where fal's own tools look for it: the
    /// environment, or a `.env` in this folder or one above it (see
    /// [`secret`]). Never from a committed file: a key in git is a key on
    /// GitHub.
    pub fn from_env() -> Result<Self> {
        let key = secret("FAL_KEY").ok_or_else(|| {
            anyhow!(
                "FAL_KEY is not set: get a key at https://fal.ai/dashboard/keys and put \
                 `FAL_KEY=…` in the environment or in a .env at the project's root"
            )
        })?;
        Ok(Self::new(key))
    }

    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(20))
                .timeout_read(Duration::from_secs(120))
                .build(),
            image_model: "fal-ai/flux/schnell".into(),
            mesh_model: "fal-ai/trellis".into(),
        }
    }

    fn auth(&self) -> String {
        format!("Key {}", self.key)
    }

    /// Submit to the queue, wait for it, and return the result.
    fn run(&self, app: &str, input: Value, progress: &mut dyn FnMut(String)) -> Result<Value> {
        let submitted: Value = self
            .agent
            .post(&format!("https://queue.fal.run/{app}"))
            .set("Authorization", &self.auth())
            .send_json(input)
            .map_err(|e| failure(app, e))?
            .into_json()?;
        let status_url = submitted["status_url"]
            .as_str()
            .ok_or_else(|| anyhow!("{app}: the queue answered without a status_url: {submitted}"))?
            .to_string();
        let response_url = submitted["response_url"]
            .as_str()
            .ok_or_else(|| anyhow!("{app}: the queue answered without a response_url"))?
            .to_string();

        let started = Instant::now();
        let mut said = String::new();
        loop {
            if started.elapsed() > Duration::from_secs(600) {
                bail!("{app}: no result after ten minutes");
            }
            let status: Value = self
                .agent
                .get(&status_url)
                .set("Authorization", &self.auth())
                .call()
                .map_err(|e| failure(app, e))?
                .into_json()?;
            let line = match status["status"].as_str() {
                Some("COMPLETED") => break,
                Some("IN_QUEUE") => match status["queue_position"].as_u64() {
                    Some(n) => format!("{app}: in queue, position {n}"),
                    None => format!("{app}: in queue"),
                },
                Some("IN_PROGRESS") => format!("{app}: running"),
                _ => bail!("{app}: unexpected status {status}"),
            };
            if line != said {
                progress(line.clone());
                said = line;
            }
            std::thread::sleep(Duration::from_millis(750));
        }
        self.agent
            .get(&response_url)
            .set("Authorization", &self.auth())
            .call()
            .map_err(|e| failure(app, e))?
            .into_json()
            .with_context(|| format!("{app}: reading the result"))
    }

    fn download(&self, url: &str) -> Result<Vec<u8>> {
        if let Some(data) = url.strip_prefix("data:") {
            let (_, encoded) = data
                .split_once(";base64,")
                .ok_or_else(|| anyhow!("a data URL that is not base64"))?;
            return Ok(base64::engine::general_purpose::STANDARD.decode(encoded)?);
        }
        let mut bytes = Vec::new();
        self.agent
            .get(url)
            .call()
            .with_context(|| format!("downloading {url}"))?
            .into_reader()
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

impl Provider for Fal {
    fn name(&self) -> &str {
        "fal"
    }

    fn generate(&self, input: &Input, progress: &mut dyn FnMut(String)) -> Result<Output> {
        let mut models = Vec::new();
        let mut seed = None;
        let mut picture = None;
        let image_url = match input {
            Input::Text(prompt) => {
                progress(format!("{}: drawing", self.image_model));
                let drawn = self.run(
                    &self.image_model,
                    json!({
                        "prompt": format!("{prompt}. {PICTURE_STYLE}"),
                        "image_size": "square_hd",
                        "num_images": 1,
                        "output_format": "png",
                    }),
                    progress,
                )?;
                models.push(self.image_model.clone());
                seed = drawn["seed"].as_u64();
                let url = drawn["images"][0]["url"]
                    .as_str()
                    .ok_or_else(|| anyhow!("{}: no image in {drawn}", self.image_model))?
                    .to_string();
                picture = self.download(&url).ok();
                url
            }
            Input::Image { bytes, mime } => format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        };

        progress(format!("{}: building the mesh", self.mesh_model));
        let built = self.run(
            &self.mesh_model,
            json!({
                "image_url": image_url,
                // The texture is only read for the draft's one colour, so
                // the smallest is the fastest and loses nothing.
                "texture_size": 512,
            }),
            progress,
        )?;
        models.push(self.mesh_model.clone());
        let url = built["model_mesh"]["url"]
            .as_str()
            .ok_or_else(|| anyhow!("{}: no model_mesh in {built}", self.mesh_model))?;
        progress("downloading the mesh".into());
        let glb = self.download(url)?;
        Ok(Output {
            glb,
            picture,
            model: models.join(" -> "),
            seed,
        })
    }
}

/// A key for a service: from the environment, or else from the nearest
/// `.env` up from the working directory that names it.
///
/// Read, not loaded: the process's environment is not written, because
/// the editor has threads and setting a variable under them is a race.
pub fn secret(name: &str) -> Option<String> {
    if let Some(value) = std::env::var(name).ok().filter(|v| !v.trim().is_empty()) {
        return Some(value);
    }
    let here = std::env::current_dir().ok()?;
    here.ancestors().find_map(|dir| {
        let text = std::fs::read_to_string(dir.join(".env")).ok()?;
        dotenv(&text, name)
    })
}

/// One variable from a `.env`'s text: `NAME=value`, with `export ` in
/// front and quotes around allowed, `#` lines skipped.
fn dotenv(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (key, value) = line.split_once('=')?;
        if line.starts_with('#') || key.trim() != name {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// An HTTP error with what the service said, not only its code: fal
/// explains a bad key or a bad argument in the body.
fn failure(app: &str, error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let hint = match code {
                401 | 403 => " — is FAL_KEY right?",
                _ => "",
            };
            anyhow!("{app}: HTTP {code}{hint}: {body}")
        }
        other => anyhow!("{app}: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::dotenv;

    #[test]
    fn a_dotenv_line_is_read_the_ways_people_write_it() {
        let text = "# keys\nOTHER=1\nexport FAL_KEY = \"abc:def\"\n";
        assert_eq!(dotenv(text, "FAL_KEY").as_deref(), Some("abc:def"));
        assert_eq!(dotenv("FAL_KEY='x'", "FAL_KEY").as_deref(), Some("x"));
        assert_eq!(dotenv("FAL_KEY=plain", "FAL_KEY").as_deref(), Some("plain"));
        assert_eq!(dotenv("#FAL_KEY=no\nFAL_KEY=", "FAL_KEY"), None);
        assert_eq!(dotenv("FAL_KEYS=no", "FAL_KEY"), None);
    }
}
