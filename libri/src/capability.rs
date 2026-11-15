//! The model each kind of making uses by default, picked in Settings →
//! Models. Answers are the catalog's pick (catalog.rs); pictures are
//! picked here, among the models this host can reach. Each choice is a
//! provider and its model, the shape hippo's summariser list shares.

use crate::model::Kept;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The picture model picked, as JSON.
pub const IMAGE: Kept = Kept("image-model");

/// A model on offer for one kind of making.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Choice {
    /// Who serves it, by its own short name ("xai").
    pub provider: String,
    /// As the provider takes it.
    pub model: String,
    /// As the page shows it.
    #[serde(default)]
    pub name: String,
}

/// The picture models this host can reach: xAI's, as its sign-in lists
/// them, when xAI is connected. Should the list fail, the model drawn
/// with by default stands in, since drawing may still work.
pub fn image_choices(state: &Path) -> Vec<Choice> {
    if crate::x::connected(state).is_none() {
        return Vec::new();
    }
    let ids = crate::x::image_models(state).unwrap_or_else(|e| {
        eprintln!("libri: image models: {e}");
        vec![crate::x::IMAGE_MODEL.to_owned()]
    });
    ids.into_iter()
        .map(|model| Choice {
            provider: "xai".into(),
            name: grok_name(&model),
            model,
        })
        .collect()
}

/// xAI's model id as its pages name it: "grok-imagine-image-2.0" is
/// "Grok Imagine 2.0", "grok-imagine-image-quality" "Grok Imagine Quality".
fn grok_name(id: &str) -> String {
    let Some(rest) = id.strip_prefix("grok-imagine-image") else {
        return id.to_owned();
    };
    let rest = rest.trim_start_matches('-');
    if rest.is_empty() {
        return "Grok Imagine".into();
    }
    let mut c = rest.chars();
    let first = c
        .next()
        .map(|f| f.to_uppercase().collect::<String>())
        .unwrap_or_default();
    format!("Grok Imagine {first}{}", c.as_str())
}

/// The picture model picked, or none for the service's own default.
pub fn image(state: &Path) -> Option<Choice> {
    IMAGE
        .read(state)
        .and_then(|j| serde_json::from_str(&j).ok())
}

/// Picks `want` for pictures, if it is among `choices`.
pub fn set_image(state: &Path, want: &Choice, choices: &[Choice]) -> Result<Choice, String> {
    let found = choices
        .iter()
        .find(|c| c.provider == want.provider && c.model == want.model)
        .ok_or_else(|| format!("{} is not on offer here", want.model))?;
    IMAGE.write(
        state,
        &serde_json::to_string(found).map_err(|e| e.to_string())?,
    )?;
    Ok(found.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_xais_pages() {
        assert_eq!(grok_name("grok-imagine-image-2.0"), "Grok Imagine 2.0");
        assert_eq!(
            grok_name("grok-imagine-image-quality"),
            "Grok Imagine Quality"
        );
        assert_eq!(grok_name("grok-imagine-image"), "Grok Imagine");
        assert_eq!(grok_name("other-model"), "other-model");
    }

    #[test]
    fn only_a_model_on_offer_is_kept() {
        let dir = std::env::temp_dir().join(format!("capability-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let on = Choice {
            provider: "xai".into(),
            model: "grok-imagine-image-2.0".into(),
            name: "Grok Imagine 2.0".into(),
        };
        let off = Choice {
            model: "made-up".into(),
            ..on.clone()
        };
        assert!(set_image(&dir, &off, std::slice::from_ref(&on)).is_err());
        assert_eq!(image(&dir), None);
        set_image(&dir, &on, std::slice::from_ref(&on)).unwrap();
        assert_eq!(image(&dir), Some(on));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
