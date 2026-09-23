//! The Inspector of an asset: its import settings, edited, and the asset
//! built again — Unity's model and texture import settings.

use std::path::Path;

use runity_import::ImportSettings;

use crate::{EditError, EditResult, Session};

/// The settings an asset's Inspector shows, by the names
/// [`Session::set_import_setting`] takes.
pub const IMPORT_FIELDS: [&str; 4] = ["scale", "recompute_normals", "srgb", "origin_to_base"];

impl Session {
    /// How a source in the project is imported: its `.rimport`, or the
    /// defaults when it has none yet.
    pub fn import_settings(&self, source: &str) -> EditResult<ImportSettings> {
        let project = self.project.as_ref().ok_or(EditError::NotInProject)?;
        let path = project.root().join(source);
        if !path.is_file() {
            return Err(EditError::Scene(format!("no {source} in the project")));
        }
        Ok(ImportSettings::load(runity_import::sidecar_for(&path))
            .unwrap_or_else(|_| ImportSettings::for_source(source)))
    }

    /// Change one import setting of a source — `scale`, `recompute_normals`,
    /// `srgb`, `origin_to_base` — write its `.rimport`, and build the asset
    /// again, so every scene drawing it shows the change at once. The
    /// sidecar is committed with the source; the built asset is not.
    pub fn set_import_setting(&mut self, source: &str, field: &str, value: &str) -> EditResult<()> {
        let mut settings = self.import_settings(source)?;
        let flag = |value: &str| -> EditResult<bool> {
            value
                .trim()
                .parse()
                .map_err(|_| EditError::Scene(format!("{field} is true or false, not `{value}`")))
        };
        match field {
            "scale" => {
                let scale: f32 = value
                    .trim()
                    .parse()
                    .map_err(|_| EditError::Scene(format!("scale is a number, not `{value}`")))?;
                if !(scale.is_finite() && scale > 0.0) {
                    return Err(EditError::Scene(format!(
                        "scale is above zero, not {scale}"
                    )));
                }
                settings.scale = scale;
            }
            "recompute_normals" => settings.recompute_normals = flag(value)?,
            "srgb" => settings.srgb = flag(value)?,
            "origin_to_base" => settings.origin_to_base = flag(value)?,
            other => {
                return Err(EditError::Scene(format!(
                    "no import setting `{other}` — there are {}",
                    IMPORT_FIELDS.join(", ")
                )))
            }
        }
        let project = self.project.clone().ok_or(EditError::NotInProject)?;
        let path = project.root().join(source);
        runity_import::import_into(&project, Path::new(&path), Some(&settings))
            .map_err(|e| EditError::Import(format!("{e:#}")))?;
        self.reopen_library()?;
        self.say(
            crate::console::Level::Info,
            format!("{source}: {field} = {value}, built again"),
        );
        Ok(())
    }
}
