use crate::settings::HomeWidgets;

pub(crate) type Field = fn(&mut HomeWidgets) -> &mut bool;

pub(crate) const WIDGETS: [(&str, &str, Field); 5] = [
    ("home-toggle-pull-requests", "Pull requests", |w| {
        &mut w.pull_requests
    }),
    ("home-toggle-git-activity", "Git activity", |w| {
        &mut w.git_activity
    }),
    ("home-toggle-this-mac", "This Mac", |w| &mut w.this_mac),
    ("home-toggle-running-services", "Running services", |w| {
        &mut w.running_services
    }),
    ("home-toggle-ci-failures", "CI failures", |w| {
        &mut w.ci_failures
    }),
];
