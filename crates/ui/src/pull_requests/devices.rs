use chrono::{DateTime, Utc};
use zeron_proto::{
    ChangeRequestFailure, ChangeRequestListError, ChangeRequestListItem, ChangeRequestListing,
    Device, Space, capabilities,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListDevice {
    pub id: String,
    pub name: String,
    pub local: bool,
}

impl ListDevice {
    pub fn target(&self) -> Option<String> {
        (!self.local).then(|| self.id.clone())
    }
}

pub(crate) fn list_devices(
    local: Option<&str>,
    devices: &[Device],
    spaces: &[Space],
    now: DateTime<Utc>,
) -> Vec<ListDevice> {
    let name_of = |id: &str| {
        devices
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.name.clone())
    };
    let mut listed = vec![ListDevice {
        id: local.unwrap_or_default().to_string(),
        name: local
            .and_then(name_of)
            .unwrap_or_else(|| "This device".into()),
        local: true,
    }];
    let mut peers: Vec<ListDevice> = devices
        .iter()
        .filter(|device| Some(device.id.as_str()) != local)
        .filter(|device| device.supports(capabilities::PULL_REQUESTS_PAGE_V1))
        .filter(|device| crate::settings::devices::device_online(device.last_seen_at, now))
        .filter(|device| {
            spaces
                .iter()
                .any(|space| space.device_id == device.id && space.git_detected)
        })
        .map(|device| ListDevice {
            id: device.id.clone(),
            name: device.name.clone(),
            local: false,
        })
        .collect();
    peers.sort_by_key(|device| device.name.to_lowercase());
    listed.extend(peers);
    listed
}

pub(crate) fn chosen_devices(listed: Vec<ListDevice>, wanted: Option<&str>) -> Vec<ListDevice> {
    let Some(wanted) = wanted else {
        return listed;
    };
    if listed.iter().any(|device| device.id == wanted) {
        listed
            .into_iter()
            .filter(|device| device.id == wanted)
            .collect()
    } else {
        listed
    }
}

pub(crate) fn viewer_key(item: &ChangeRequestListItem) -> String {
    match &item.device_id {
        Some(device) => format!("{device} {}", item.host),
        None => item.host.clone(),
    }
}

pub(crate) fn merge_listings(
    answers: Vec<(&ListDevice, Result<ChangeRequestListing, String>)>,
) -> Result<ChangeRequestListing, String> {
    let mut merged = ChangeRequestListing::default();
    let mut first_error = None;
    let mut any_ok = false;
    for (device, answer) in answers {
        match answer {
            Ok(listing) => {
                any_ok = true;
                let target = device.target();
                for (host, login) in listing.viewers {
                    let key = match &target {
                        Some(id) => format!("{id} {host}"),
                        None => host,
                    };
                    merged.viewers.insert(key, login);
                }
                merged
                    .items
                    .extend(listing.items.into_iter().map(|mut item| {
                        item.device_id = target.clone();
                        item
                    }));
                merged.errors.extend(listing.errors);
                merged.truncated |= listing.truncated;
            }
            Err(error) => {
                if !device.local {
                    merged.errors.push(ChangeRequestListError {
                        host: None,
                        repository: None,
                        path: device.name.clone(),
                        failure: ChangeRequestFailure::CommandFailed,
                    });
                }
                first_error.get_or_insert(error);
            }
        }
    }
    if !any_ok {
        return Err(first_error.unwrap_or_default());
    }
    merged
        .items
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, online: bool, capable: bool) -> Device {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "platform": "macos",
            "lastSeenAt": if online { Utc::now().to_rfc3339() } else { "2020-01-01T00:00:00Z".into() },
            "capabilities": if capable { vec![capabilities::PULL_REQUESTS_PAGE_V1] } else { vec![] },
        }))
        .unwrap()
    }

    fn space(device: &str, git: bool) -> Space {
        serde_json::from_value(serde_json::json!({
            "id": format!("space-{device}"),
            "deviceId": device,
            "path": "/repo",
            "gitDetected": git,
            "createdAt": "2026-10-01T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn peers_need_to_be_online_capable_and_hold_a_git_project() {
        let devices = [
            device("mac", "Mac", true, true),
            device("studio", "Studio", true, true),
            device("old", "Old", true, false),
            device("away", "Away", false, true),
            device("plain", "Plain", true, true),
            device("air", "air", true, true),
        ];
        let spaces = [
            space("studio", true),
            space("old", true),
            space("away", true),
            space("plain", false),
            space("air", true),
        ];
        let listed = list_devices(Some("mac"), &devices, &spaces, Utc::now());
        let names: Vec<_> = listed.iter().map(|d| (d.name.as_str(), d.local)).collect();
        assert_eq!(names, [("Mac", true), ("air", false), ("Studio", false)]);
        assert_eq!(listed[0].target(), None);
        let names =
            |chosen: Vec<ListDevice>| chosen.into_iter().map(|d| d.name).collect::<Vec<_>>();
        assert_eq!(
            names(chosen_devices(listed.clone(), Some("studio"))),
            ["Studio"]
        );
        assert_eq!(names(chosen_devices(listed.clone(), Some("away"))).len(), 3);
        assert_eq!(listed[2].target().as_deref(), Some("studio"));
    }

    #[test]
    fn merged_rows_carry_their_device_and_a_lost_peer_becomes_an_error_row() {
        let local = ListDevice {
            id: "mac".into(),
            name: "Mac".into(),
            local: true,
        };
        let studio = ListDevice {
            id: "studio".into(),
            name: "Studio".into(),
            local: false,
        };
        let gone = ListDevice {
            id: "gone".into(),
            name: "Gone".into(),
            local: false,
        };
        let listing = |login: &str, number: u64, updated: &str| -> ChangeRequestListing {
            serde_json::from_value(serde_json::json!({
                "viewers": {"github.com": login},
                "items": [{
                    "provider": "github", "host": "github.com", "repository": "acme/zeron",
                    "number": number, "title": "t", "url": "u", "headRef": "h", "baseRef": "b",
                    "state": "open", "isDraft": false, "mergeability": "unknown",
                    "additions": 0, "deletions": 0, "createdAt": updated, "updatedAt": updated,
                    "viewerReviewRequested": false, "labels": []
                }],
                "errors": [],
                "truncated": number == 2
            }))
            .unwrap()
        };
        let merged = merge_listings(vec![
            (&local, Ok(listing("alice", 1, "2026-10-01T00:00:00Z"))),
            (&studio, Ok(listing("bob", 2, "2026-10-02T00:00:00Z"))),
            (&gone, Err("closed".into())),
        ])
        .unwrap();
        let rows: Vec<_> = merged
            .items
            .iter()
            .map(|item| (item.number, item.device_id.as_deref()))
            .collect();
        assert_eq!(rows, [(2, Some("studio")), (1, None)]);
        assert_eq!(merged.viewers["github.com"], "alice");
        assert_eq!(merged.viewers["studio github.com"], "bob");
        assert_eq!(viewer_key(&merged.items[0]), "studio github.com");
        assert!(merged.truncated);
        assert_eq!(merged.errors.len(), 1);
        assert_eq!(merged.errors[0].path, "Gone");
        assert_eq!(
            merge_listings(vec![(&local, Err("offline".into()))]),
            Err("offline".to_string())
        );
    }
}
