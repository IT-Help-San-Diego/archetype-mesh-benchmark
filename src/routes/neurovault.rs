//! NeuroVault integration — brain visualization data from neurovault.org.
//! Wired into the router (2026-07-14): /api/neurovault/{collections,images,manifest}
//! feed the science layer beside the hero brain.

use axum::{extract::Path, response::Json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuroImage {
    pub id: i64,
    pub name: String,
    pub map_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuroCollection {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub doi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_url: Option<String>,
    pub image_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub collection_id: i64,
    pub image_id: i64,
    pub doi: Option<String>,
    pub paper_url: Option<String>,
    pub map_type: String,
}

fn collections_data() -> Vec<NeuroCollection> {
    vec![
        NeuroCollection {
            id: 22786,
            name: "Bayesian social and ToM reasoning".into(),
            description: Some("Whole-brain contrasts for theory-of-mind and social Bayesian reasoning".into()),
            doi: Some("10.1038/s41467-026-71151-2".into()),
            paper_url: Some("https://www.nature.com/articles/s41467-026-71151-2".into()),
            image_count: 2,
        },
        NeuroCollection {
            id: 21999,
            name: "Emotional and valence-driven memory decisions".into(),
            description: Some("Decision-making memory contrasts: negative and positive valence".into()),
            doi: Some("10.1162/IMAG.a.1213".into()),
            paper_url: Some("https://doi.org/10.1162/IMAG.a.1213".into()),
            image_count: 3,
        },
        NeuroCollection {
            id: 21877,
            name: "Serotonin receptor binding distributions".into(),
            description: Some("5HT1b, 5HT2a, and 5HT4 receptor imaging maps".into()),
            doi: Some("10.1016/j.pnpbp.2026.111679".into()),
            paper_url: Some("https://doi.org/10.1016/j.pnpbp.2026.111679".into()),
            image_count: 3,
        },
    ]
}

fn images_data() -> std::collections::HashMap<i64, Vec<NeuroImage>> {
    let mut map = std::collections::HashMap::new();
    map.insert(22786, vec![
        NeuroImage {
            id: 1023939,
            name: "Social ToM Bayes".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/22786/1023939_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/22786/1023939_1.nii.gz".into()),
        },
        NeuroImage {
            id: 1023941,
            name: "Social ToM Text Bayes".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/22786/1023941_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/22786/1023941_1.nii.gz".into()),
        },
    ]);
    map.insert(21999, vec![
        NeuroImage {
            id: 1011028,
            name: "Emotional DM".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/21999/1011028_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/21999/1011028_1.nii.gz".into()),
        },
        NeuroImage {
            id: 1011031,
            name: "Negative DM".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/21999/1011031_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/21999/1011031_1.nii.gz".into()),
        },
        NeuroImage {
            id: 1011032,
            name: "Positive DM".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/21999/1011032_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/21999/1011032_1.nii.gz".into()),
        },
    ]);
    map.insert(21877, vec![
        NeuroImage {
            id: 1010060,
            name: "5HT1b".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/21877/1010060_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/21877/1010060_1.nii.gz".into()),
        },
        NeuroImage {
            id: 1010509,
            name: "5HT2a".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/21877/1010509_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/21877/1010509_1.nii.gz".into()),
        },
        NeuroImage {
            id: 1010511,
            name: "5HT4".into(),
            map_type: "statistical map".into(),
            thumbnail_url: Some("https://neurovault.org/media/images/21877/1010511_1_thumb.jpg".into()),
            file_url: Some("https://neurovault.org/media/images/21877/1010511_1.nii.gz".into()),
        },
    ]);
    map
}

pub async fn neurovault_collections() -> Json<Vec<NeuroCollection>> {
    Json(collections_data())
}

pub async fn neurovault_images(Path(collection_id): Path<i64>) -> Json<Vec<NeuroImage>> {
    let images = images_data().get(&collection_id).cloned().unwrap_or_default();
    Json(images)
}

pub async fn neurovault_manifest() -> Json<Vec<ManifestEntry>> {
    let images = images_data();
    let collections = collections_data();
    let mut out = Vec::new();
    for col in collections {
        if let Some(imgs) = images.get(&col.id) {
            for img in imgs {
                out.push(ManifestEntry {
                    collection_id: col.id,
                    image_id: img.id,
                    doi: col.doi.clone(),
                    paper_url: col.paper_url.clone(),
                    map_type: img.map_type.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.collection_id.cmp(&b.collection_id).then(a.image_id.cmp(&b.image_id)));
    Json(out)
}
