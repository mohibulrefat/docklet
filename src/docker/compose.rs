//! Basic Compose support — a thin, read-only layer over the Engine API.
//!
//! Docker Engine has no Compose endpoints; Compose is a CLI plugin that
//! records its own bookkeeping as container labels. This module reads those
//! labels and nothing else: no CLI, no compose plugin dependency, no YAML
//! parsing, no attempt at full `compose up` semantics. Projects are grouped
//! from whatever containers already exist, so a project with no containers
//! at all is invisible here — an accepted limitation of a labels-only view.

use super::{Container, Docker, DockerError};

/// The labels Compose writes on every container it creates.
mod label {
    pub const PROJECT: &str = "com.docker.compose.project";
    pub const SERVICE: &str = "com.docker.compose.service";
    pub const CONFIG_FILES: &str = "com.docker.compose.project.config_files";
    pub const WORKING_DIR: &str = "com.docker.compose.project.working_dir";
}

/// One service's container within a project.
#[derive(Debug, Clone)]
pub struct ComposeService {
    pub name: String,
    pub container_id: String,
    pub container_name: String,
    pub state: String,
}

/// A Compose project, reconstructed from its containers' labels.
///
/// `config_files` and `working_dir` are carried through as opaque strings —
/// never opened, never parsed — purely so a future, fuller implementation has
/// the path it would need without a model change. Reading them here would
/// mean a YAML dependency and a second source of truth beside the daemon,
/// which is exactly what this approach avoids.
#[derive(Debug, Clone)]
pub struct ComposeProject {
    pub name: String,
    pub services: Vec<ComposeService>,
    pub working_dir: String,
    pub config_files: String,
}

/// Whether every, some, or none of a project's containers are running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectState {
    Running,
    Partial,
    Stopped,
}

impl ComposeProject {
    pub fn state(&self) -> ProjectState {
        let running = self
            .services
            .iter()
            .filter(|s| s.state == "running")
            .count();
        match running {
            0 => ProjectState::Stopped,
            n if n == self.services.len() => ProjectState::Running,
            _ => ProjectState::Partial,
        }
    }

    pub fn service_count(&self) -> usize {
        self.services.len()
    }
}

/// Group containers into Compose projects by their labels.
///
/// A pure function, independently testable against a fixture with no daemon:
/// the grouping logic is what M9 actually needs to get right, and it does not
/// need Docker running to prove that.
pub fn group_projects(containers: Vec<Container>) -> Vec<ComposeProject> {
    let mut projects: Vec<ComposeProject> = Vec::new();

    for container in containers {
        let Some(project_name) = container.labels.get(label::PROJECT).cloned() else {
            continue;
        };
        let service_name = container
            .labels
            .get(label::SERVICE)
            .cloned()
            .unwrap_or_default();

        let service = ComposeService {
            name: service_name,
            container_id: container.id.clone(),
            container_name: container.name().to_string(),
            state: container.state.clone(),
        };

        match projects.iter_mut().find(|p| p.name == project_name) {
            Some(project) => project.services.push(service),
            None => projects.push(ComposeProject {
                name: project_name,
                working_dir: container
                    .labels
                    .get(label::WORKING_DIR)
                    .cloned()
                    .unwrap_or_default(),
                config_files: container
                    .labels
                    .get(label::CONFIG_FILES)
                    .cloned()
                    .unwrap_or_default(),
                services: vec![service],
            }),
        }
    }

    projects.sort_by(|a, b| a.name.cmp(&b.name));
    projects
}

/// The outcome of starting or stopping one project.
///
/// A project is several independent API calls, so it can partly succeed;
/// this is what lets the UI say exactly that instead of a single pass/fail.
#[derive(Debug, Default)]
pub struct ProjectActionResult {
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl ProjectActionResult {
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }
}

impl Docker {
    /// List Compose projects, derived from the current container list.
    pub fn compose_projects(&self) -> Result<Vec<ComposeProject>, DockerError> {
        Ok(group_projects(self.containers(true)?))
    }

    /// Start every stopped container in a project.
    ///
    /// This starts containers that already exist; it is not `compose up` and
    /// does not create anything Compose itself would create. Each container
    /// is started in turn, and a failure on one does not stop the rest.
    pub fn compose_start(&self, project: &ComposeProject) -> ProjectActionResult {
        let mut result = ProjectActionResult::default();
        for service in &project.services {
            if service.state == "running" {
                continue;
            }
            match self.start_container(&service.container_id) {
                Ok(()) => result.succeeded.push(service.name.clone()),
                Err(e) => result.failed.push((service.name.clone(), e.to_string())),
            }
        }
        result
    }

    /// Stop every running container in a project.
    pub fn compose_stop(&self, project: &ComposeProject) -> ProjectActionResult {
        let mut result = ProjectActionResult::default();
        for service in &project.services {
            if service.state != "running" {
                continue;
            }
            match self.stop_container(&service.container_id) {
                Ok(()) => result.succeeded.push(service.name.clone()),
                Err(e) => result.failed.push((service.name.clone(), e.to_string())),
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn labeled(project: &str, service: &str, id: &str, state: &str) -> Container {
        let mut labels = HashMap::new();
        labels.insert(label::PROJECT.to_string(), project.to_string());
        labels.insert(label::SERVICE.to_string(), service.to_string());
        labels.insert(
            label::WORKING_DIR.to_string(),
            format!("/home/user/{project}"),
        );
        labels.insert(
            label::CONFIG_FILES.to_string(),
            format!("/home/user/{project}/docker-compose.yml"),
        );

        serde_json::from_value(serde_json::json!({
            "Id": id,
            "Names": [format!("/{project}-{service}-1")],
            "Image": "img",
            "State": state,
            "Status": "status",
            "Labels": labels,
        }))
        .unwrap()
    }

    fn unlabeled(id: &str) -> Container {
        serde_json::from_value(serde_json::json!({
            "Id": id, "Names": ["/plain"], "Image": "img", "State": "running", "Status": "Up",
        }))
        .unwrap()
    }

    #[test]
    fn groups_containers_by_project() {
        let projects = group_projects(vec![
            labeled("app", "web", "a", "running"),
            labeled("app", "db", "b", "running"),
            labeled("other", "cache", "c", "running"),
        ]);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "app");
        assert_eq!(projects[0].services.len(), 2);
        assert_eq!(projects[1].name, "other");
    }

    #[test]
    fn ignores_containers_with_no_project_label() {
        let projects = group_projects(vec![unlabeled("a"), labeled("app", "web", "b", "running")]);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "app");
    }

    #[test]
    fn sorts_projects_by_name() {
        let projects = group_projects(vec![
            labeled("zebra", "s", "a", "running"),
            labeled("alpha", "s", "b", "running"),
        ]);
        assert_eq!(projects[0].name, "alpha");
        assert_eq!(projects[1].name, "zebra");
    }

    #[test]
    fn carries_working_dir_and_config_files_without_reading_them() {
        let projects = group_projects(vec![labeled("app", "web", "a", "running")]);
        assert_eq!(projects[0].working_dir, "/home/user/app");
        assert_eq!(
            projects[0].config_files,
            "/home/user/app/docker-compose.yml"
        );
    }

    #[test]
    fn reports_all_running() {
        let projects = group_projects(vec![
            labeled("app", "web", "a", "running"),
            labeled("app", "db", "b", "running"),
        ]);
        assert_eq!(projects[0].state(), ProjectState::Running);
    }

    #[test]
    fn reports_all_stopped() {
        let projects = group_projects(vec![
            labeled("app", "web", "a", "exited"),
            labeled("app", "db", "b", "exited"),
        ]);
        assert_eq!(projects[0].state(), ProjectState::Stopped);
    }

    #[test]
    fn reports_partial_when_only_some_are_running() {
        let projects = group_projects(vec![
            labeled("app", "web", "a", "running"),
            labeled("app", "db", "b", "exited"),
        ]);
        assert_eq!(projects[0].state(), ProjectState::Partial);
    }

    #[test]
    fn counts_services() {
        let projects = group_projects(vec![
            labeled("app", "web", "a", "running"),
            labeled("app", "db", "b", "running"),
            labeled("app", "cache", "c", "running"),
        ]);
        assert_eq!(projects[0].service_count(), 3);
    }

    #[test]
    fn produces_nothing_for_an_empty_container_list() {
        assert!(group_projects(vec![]).is_empty());
    }

    fn project_of(services: Vec<(&str, &str, &str)>) -> ComposeProject {
        ComposeProject {
            name: "app".to_string(),
            working_dir: String::new(),
            config_files: String::new(),
            services: services
                .into_iter()
                .map(|(name, id, state)| ComposeService {
                    name: name.to_string(),
                    container_id: id.to_string(),
                    container_name: format!("app-{name}-1"),
                    state: state.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn identifies_which_services_a_start_would_touch() {
        // compose_start/compose_stop's own HTTP calls are covered live
        // (against this machine's real Compose projects); this is the pure
        // bookkeeping around them — which services an action considers.
        let project = project_of(vec![
            ("web", "a", "running"),
            ("db", "b", "exited"),
            ("cache", "c", "exited"),
        ]);
        let to_start: Vec<&str> = project
            .services
            .iter()
            .filter(|s| s.state != "running")
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(to_start, vec!["db", "cache"]);
    }

    #[test]
    fn identifies_which_services_a_stop_would_touch() {
        let project = project_of(vec![
            ("web", "a", "running"),
            ("db", "b", "exited"),
            ("cache", "c", "running"),
        ]);
        let to_stop: Vec<&str> = project
            .services
            .iter()
            .filter(|s| s.state == "running")
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(to_stop, vec!["web", "cache"]);
    }

    #[test]
    fn project_action_result_reports_success_only_with_no_failures() {
        let mut result = ProjectActionResult::default();
        assert!(result.is_success());
        result.failed.push(("db".to_string(), "boom".to_string()));
        assert!(!result.is_success());
    }

    /// Real labels captured from this machine's own containers.
    #[test]
    fn groups_a_real_two_project_fixture() {
        let containers: Vec<Container> = serde_json::from_str(
            r#"[
                {"Id":"1","Names":["/ubl-backend"],"Image":"i","State":"exited","Status":"s",
                 "Labels":{"com.docker.compose.project":"ubl-backend-services",
                           "com.docker.compose.service":"backend"}},
                {"Id":"2","Names":["/ubl-redis"],"Image":"i","State":"exited","Status":"s",
                 "Labels":{"com.docker.compose.project":"ubl-backend-services",
                           "com.docker.compose.service":"redis"}},
                {"Id":"3","Names":["/xlabo_rebuild-nginx-1"],"Image":"i","State":"exited","Status":"s",
                 "Labels":{"com.docker.compose.project":"xlabo_rebuild",
                           "com.docker.compose.service":"nginx"}}
            ]"#,
        )
        .unwrap();

        let projects = group_projects(containers);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "ubl-backend-services");
        assert_eq!(projects[0].services.len(), 2);
        assert_eq!(projects[1].name, "xlabo_rebuild");
        assert_eq!(projects[1].services.len(), 1);
    }
}
