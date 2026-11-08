//! Lossless string interning on the wire; callers keep ordinary computed maps.
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
use std::collections::BTreeMap;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    strings: Vec<String>,
    maps: Vec<Vec<(usize, usize)>>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Wire {
    Legacy(Vec<BTreeMap<String, String>>),
    Compact(Table),
}
pub fn serialize<S: Serializer>(
    maps: &[BTreeMap<String, String>],
    s: S,
) -> Result<S::Ok, S::Error> {
    let mut ids: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut strings = Vec::new();
    let mut intern = |value: &str| {
        if let Some(id) = ids.get(value) {
            *id
        } else {
            let id = strings.len();
            strings.push(value.to_owned());
            ids.insert(value.to_owned(), id);
            id
        }
    };
    let maps = maps
        .iter()
        .map(|m| m.iter().map(|(k, v)| (intern(k), intern(v))).collect())
        .collect();
    Table { strings, maps }.serialize(s)
}
pub fn deserialize<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Vec<BTreeMap<String, String>>, D::Error> {
    match Wire::deserialize(d)? {
        Wire::Legacy(maps) => Ok(maps),
        Wire::Compact(table) => {
            if table.maps.len() > 4096 || table.strings.len() > 200_000 {
                return Err(D::Error::custom("compact style table limit"));
            }
            let mut bytes = 0usize;
            table
                .maps
                .into_iter()
                .map(|map| {
                    if map.len() > 2048 {
                        return Err(D::Error::custom("compact style field limit"));
                    }
                    let mut result = BTreeMap::new();
                    for (k, v) in map {
                        let k = table
                            .strings
                            .get(k)
                            .ok_or_else(|| D::Error::custom("compact style key reference"))?;
                        let v = table
                            .strings
                            .get(v)
                            .ok_or_else(|| D::Error::custom("compact style value reference"))?;
                        bytes = bytes.saturating_add(k.len() + v.len());
                        if bytes > 128 * 1024 * 1024 {
                            return Err(D::Error::custom("expanded style byte limit"));
                        }
                        if result.insert(k.clone(), v.clone()).is_some() {
                            return Err(D::Error::custom("duplicate compact style property"));
                        }
                    }
                    Ok(result)
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Example {
        #[serde(with = "super")]
        styles: Vec<BTreeMap<String, String>>,
    }
    #[test]
    fn lossless_shared_strings_and_invalid_references() {
        let maps = (0..200)
            .map(|n| {
                BTreeMap::from([
                    ("color".into(), "rgb(12, 34, 56)".into()),
                    ("--repeated".into(), "shared-value".repeat(100)),
                    ("width".into(), format!("{n}px")),
                ])
            })
            .collect::<Vec<_>>();
        let legacy = serde_json::to_vec(&maps).unwrap();
        let encoded = serde_json::to_vec(&Example {
            styles: maps.clone(),
        })
        .unwrap();
        assert!(encoded.len() < legacy.len() / 10);
        assert_eq!(
            serde_json::from_slice::<Example>(&encoded).unwrap().styles,
            maps
        );
        assert_eq!(
            serde_json::from_value::<Example>(serde_json::json!({"styles":maps}))
                .unwrap()
                .styles,
            maps
        );
        for maps in [
            serde_json::json!([[[0, 9]]]),
            serde_json::json!([[[0, 1], [0, 1]]]),
        ] {
            assert!(
                serde_json::from_value::<Example>(
                    serde_json::json!({"styles":{"strings":["color","red"],"maps":maps}})
                )
                .is_err()
            );
        }
    }
}
