//! Git history lanes: commits in `git log --topo-order` order laid out as
//! a lane graph, each row with its node lane and the segments through it.
use std::collections::HashSet;

use crate::GitHistoryCommit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentShape {
    Through,
    Incoming,
    Outgoing,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSegment {
    pub from_lane: usize,
    pub to_lane: usize,
    pub color_id: usize,
    pub shape: SegmentShape,
    pub dashed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub sha: String,
    pub node_lane: usize,
    pub node_color_id: usize,
    pub segments: Vec<GraphSegment>,
    pub is_head: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphLayout {
    pub rows: Vec<GraphRow>,
    pub max_lane_count: usize,
}

pub const WORKING_TREE_ROW: &str = "working-tree";

#[derive(Debug, Clone)]
struct ActiveLane {
    id: usize,
    color_id: usize,
    target_sha: String,
    dashed: bool,
}

fn lane_index(lanes: &[ActiveLane], id: usize) -> usize {
    lanes
        .iter()
        .position(|lane| lane.id == id)
        .expect("active Git history lane exists")
}

/// Commits arrive child-before-parent from `git log --topo-order`. Active
/// lanes point at the parent commit that will eventually resolve each path.
pub fn layout_graph(commits: &[GitHistoryCommit], head_sha: Option<&str>) -> GraphLayout {
    let mut active_lanes: Vec<ActiveLane> = Vec::new();
    let mut next_lane_id = 0usize;
    let mut next_color_id = 0usize;
    let mut max_lane_count = 0usize;
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let before = active_lanes.clone();
        let incoming: Vec<usize> = before
            .iter()
            .enumerate()
            .filter_map(|(index, lane)| (lane.target_sha == commit.sha).then_some(index))
            .collect();
        let primary_incoming = incoming.first().copied();
        let node_lane = primary_incoming.unwrap_or(before.len());
        let primary_lane = primary_incoming.and_then(|index| before.get(index));
        let node_color_id = primary_lane.map(|lane| lane.color_id).unwrap_or_else(|| {
            let color = next_color_id;
            next_color_id += 1;
            color
        });
        let resolved_ids: HashSet<usize> = incoming.iter().map(|&index| before[index].id).collect();
        let mut next_lanes: Vec<ActiveLane> = before
            .iter()
            .filter(|lane| !resolved_ids.contains(&lane.id))
            .cloned()
            .collect();
        let mut outgoing: Vec<(usize, usize, bool)> = Vec::new();

        let mut primary_outgoing_id = None;
        if let Some(first_parent) = commit.parent_shas.first() {
            let id = primary_lane.map(|lane| lane.id).unwrap_or_else(|| {
                let id = next_lane_id;
                next_lane_id += 1;
                id
            });
            let lane = ActiveLane {
                id,
                color_id: node_color_id,
                target_sha: first_parent.clone(),
                dashed: commit.sha == WORKING_TREE_ROW,
            };
            next_lanes.insert(node_lane.min(next_lanes.len()), lane.clone());
            primary_outgoing_id = Some(id);
            outgoing.push((id, lane.color_id, lane.dashed));
        }

        let mut parent_offset = 1usize;
        for parent_sha in commit.parent_shas.iter().skip(1) {
            if let Some(existing) = next_lanes
                .iter()
                .find(|lane| lane.target_sha == *parent_sha)
            {
                outgoing.push((existing.id, existing.color_id, false));
                continue;
            }
            let lane = ActiveLane {
                id: next_lane_id,
                color_id: next_color_id,
                target_sha: parent_sha.clone(),
                dashed: false,
            };
            next_lane_id += 1;
            next_color_id += 1;
            let primary_index = primary_outgoing_id
                .map(|id| lane_index(&next_lanes, id))
                .unwrap_or_else(|| node_lane.min(next_lanes.len()));
            next_lanes.insert(
                (primary_index + parent_offset).min(next_lanes.len()),
                lane.clone(),
            );
            parent_offset += 1;
            outgoing.push((lane.id, lane.color_id, false));
        }

        let mut segments: Vec<GraphSegment> = before
            .iter()
            .enumerate()
            .filter(|(_, lane)| !resolved_ids.contains(&lane.id))
            .map(|(from_lane, lane)| GraphSegment {
                from_lane,
                to_lane: lane_index(&next_lanes, lane.id),
                color_id: lane.color_id,
                shape: SegmentShape::Through,
                dashed: lane.dashed,
            })
            .collect();
        segments.extend(incoming.iter().map(|&from_lane| GraphSegment {
            from_lane,
            to_lane: node_lane,
            color_id: before[from_lane].color_id,
            shape: SegmentShape::Incoming,
            dashed: before[from_lane].dashed,
        }));
        segments.extend(
            outgoing
                .into_iter()
                .map(|(id, color_id, dashed)| GraphSegment {
                    from_lane: node_lane,
                    to_lane: lane_index(&next_lanes, id),
                    color_id,
                    shape: SegmentShape::Outgoing,
                    dashed,
                }),
        );

        max_lane_count = max_lane_count
            .max(before.len())
            .max(next_lanes.len())
            .max(node_lane + 1);
        rows.push(GraphRow {
            sha: commit.sha.clone(),
            node_lane,
            node_color_id,
            segments,
            is_head: head_sha == Some(commit.sha.as_str()),
        });
        active_lanes = next_lanes;
    }

    GraphLayout {
        rows,
        max_lane_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, parents: &[&str]) -> GitHistoryCommit {
        GitHistoryCommit {
            sha: sha.into(),
            parent_shas: parents.iter().map(|value| (*value).into()).collect(),
            subject: sha.into(),
            author_name: "Test".into(),
            author_email: "test@example.com".into(),
            authored_at: "2026-08-12T12:00:00Z".into(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn graph_splits_and_rejoins_merge_lanes() {
        let commits = vec![
            commit("merge", &["main", "feature"]),
            commit("main", &["base"]),
            commit("feature", &["base"]),
            commit("base", &[]),
        ];
        let graph = layout_graph(&commits, Some("merge"));
        assert_eq!(graph.rows.len(), commits.len());
        assert!(graph.max_lane_count >= 2);
        assert!(graph.rows[0].is_head);
        assert_eq!(graph.rows[0].segments.len(), 2);
        assert_eq!(graph.rows[3].segments.len(), 2);
        assert_eq!(graph.rows[3].node_lane, 0);
    }

    #[test]
    fn appending_older_commits_preserves_the_loaded_prefix_layout() {
        let prefix = vec![commit("tip", &["parent"]), commit("parent", &["root"])];
        let before = layout_graph(&prefix, Some("tip"));
        let mut all = prefix.clone();
        all.push(commit("root", &[]));
        let after = layout_graph(&all, Some("tip"));
        assert_eq!(before.rows, after.rows[..before.rows.len()]);
    }
}
