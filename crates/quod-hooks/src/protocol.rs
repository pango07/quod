//! The container-hooks protocol, exactly as actions/runner speaks it
//! (`src/Runner.Worker/Container/ContainerHooks/HookInput.cs`,
//! `HookResponse.cs`; `packages/hooklib/src/interfaces.ts` in
//! actions/runner-container-hooks v0.8.1).
//!
//! The runner starts the hook script with its bundled node, writes one JSON
//! document to stdin and reads the response, if the command has one, from
//! `responseFile`. Exit 0 is success; anything else fails the step (or the
//! job, for `prepare_job`). Everything the hook prints goes to the step log
//! and through the runner's `::` command processing.

use std::collections::BTreeMap;
use std::io::Read;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    #[serde(rename = "prepare_job")]
    PrepareJob,
    #[serde(rename = "cleanup_job")]
    CleanupJob,
    #[serde(rename = "run_script_step")]
    RunScriptStep,
    #[serde(rename = "run_container_step")]
    RunContainerStep,
}

/// One hook invocation. `args` is decoded per command (see `Input::args`);
/// `state` is whatever the previous response's `state` was, or empty.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub command: Command,
    #[serde(default)]
    pub response_file: Option<String>,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub state: serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mount {
    pub source_volume_path: String,
    pub target_volume_path: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub server_url: Option<String>,
}

/// A job, service or step container as the runner describes it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub dockerfile: Option<String>,
    #[serde(default)]
    pub entry_point: Option<String>,
    #[serde(default)]
    pub entry_point_args: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub create_options: Option<String>,
    #[serde(default)]
    pub environment_variables: BTreeMap<String, String>,
    #[serde(default)]
    pub prepend_path: Vec<String>,
    #[serde(default)]
    pub user_mount_volumes: Vec<Mount>,
    #[serde(default)]
    pub system_mount_volumes: Vec<Mount>,
    #[serde(default)]
    pub registry: Option<Registry>,
    #[serde(default)]
    pub port_mappings: Vec<String>,
    /// Services only: the name in the job context.
    #[serde(default)]
    pub context_name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareJobArgs {
    #[serde(default)]
    pub container: Option<Container>,
    #[serde(default)]
    pub services: Vec<Container>,
}

/// A `run:` step or a JavaScript action inside the job container: the
/// runner's `docker exec` equivalent. `entry_point_args` is the runner's
/// argument string split on spaces (it does the splitting).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptStepArgs {
    pub entry_point: String,
    #[serde(default)]
    pub entry_point_args: Vec<String>,
    #[serde(default)]
    pub environment_variables: BTreeMap<String, String>,
    #[serde(default)]
    pub prepend_path: Vec<String>,
    pub working_directory: String,
}

impl Input {
    /// Reads one hook input from `r` (stdin).
    pub fn read(r: &mut impl Read) -> Result<Input, ProtocolError> {
        let mut s = String::new();
        r.read_to_string(&mut s)?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn prepare_job_args(&self) -> Result<PrepareJobArgs, ProtocolError> {
        Ok(serde_json::from_value(self.args.clone())?)
    }

    pub fn script_step_args(&self) -> Result<ScriptStepArgs, ProtocolError> {
        Ok(serde_json::from_value(self.args.clone())?)
    }

    pub fn container_step_args(&self) -> Result<Container, ProtocolError> {
        Ok(serde_json::from_value(self.args.clone())?)
    }

    /// `state.<key>` as a string, if present.
    pub fn state_str(&self, key: &str) -> Option<&str> {
        self.state.get(key).and_then(|v| v.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("reading hook input: {0}")]
    Io(#[from] std::io::Error),
    #[error("hook input is not the runner's JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// What `prepare_job` answers. `is_alpine` is required whenever a job
/// container was given (the runner picks its node build from it).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareJobResponse {
    pub state: serde_json::Value,
    pub context: ResponseContext,
    pub is_alpine: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<ResponseContainer>,
    pub services: Vec<ResponseContainer>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseContainer {
    pub id: String,
    pub network: String,
    pub ports: BTreeMap<String, String>,
}

/// The response of the other commands: only `state` is read.
#[derive(Clone, Debug, Default, Serialize)]
pub struct StateResponse {
    pub state: serde_json::Value,
}

/// Writes a response where the runner asked for it. The runner creates the
/// file empty before the hook starts and deletes it after reading.
pub fn write_response(
    response_file: Option<&str>,
    body: &impl Serialize,
) -> Result<(), ProtocolError> {
    if let Some(p) = response_file {
        std::fs::write(p, serde_json::to_vec(body)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `prepare_job` example from actions/runner's ADR 1891, as the
    /// runner serializes it (camelCase, `container` not `jobContainer`).
    const PREPARE: &str = r#"{
      "command": "prepare_job",
      "responseFile": "/runner/_work/_temp/_runner_hook_responses/x.json",
      "state": {},
      "args": {
        "container": {
          "image": "node:14.16",
          "workingDirectory": "/__w/thboop-test2/thboop-test2",
          "createOptions": "--cpus 1",
          "environmentVariables": {"NODE_ENV": "development", "HOME": "/github/home"},
          "userMountVolumes": [{"sourceVolumePath": "my_docker_volume", "targetVolumePath": "/volume_mount", "readOnly": false}],
          "systemMountVolumes": [
            {"sourceVolumePath": "/runner/_work", "targetVolumePath": "/__w", "readOnly": false},
            {"sourceVolumePath": "/runner/externals", "targetVolumePath": "/__e", "readOnly": true}
          ],
          "registry": null,
          "portMappings": ["8080:80/tcp"]
        },
        "services": []
      }
    }"#;

    const SCRIPT: &str = r#"{
      "command": "run_script_step",
      "responseFile": "/runner/_work/_temp/_runner_hook_responses/y.json",
      "state": {"job": "01J", "network": ""},
      "args": {
        "entryPointArgs": ["-e", "/__w/_temp/abc.sh"],
        "entryPoint": "sh",
        "environmentVariables": {"GITHUB_RUN_ID": "1", "PATH": "/usr/bin"},
        "prependPath": ["/__w/_tool/go/1.27.3/arm64/bin"],
        "workingDirectory": "/__w/o/r"
      }
    }"#;

    #[test]
    fn parses_prepare_job() {
        let i = Input::read(&mut PREPARE.as_bytes()).unwrap();
        assert_eq!(i.command, Command::PrepareJob);
        let a = i.prepare_job_args().unwrap();
        let c = a.container.unwrap();
        assert_eq!(c.image.as_deref(), Some("node:14.16"));
        assert_eq!(
            c.working_directory.as_deref(),
            Some("/__w/thboop-test2/thboop-test2")
        );
        assert_eq!(c.environment_variables["HOME"], "/github/home");
        assert_eq!(c.system_mount_volumes.len(), 2);
        assert!(c.system_mount_volumes[1].read_only);
        assert_eq!(c.user_mount_volumes[0].target_volume_path, "/volume_mount");
        assert!(a.services.is_empty());
    }

    #[test]
    fn parses_script_step_and_state() {
        let i = Input::read(&mut SCRIPT.as_bytes()).unwrap();
        assert_eq!(i.command, Command::RunScriptStep);
        assert_eq!(i.state_str("job"), Some("01J"));
        let a = i.script_step_args().unwrap();
        assert_eq!(a.entry_point, "sh");
        assert_eq!(a.entry_point_args, ["-e", "/__w/_temp/abc.sh"]);
        assert_eq!(a.prepend_path, ["/__w/_tool/go/1.27.3/arm64/bin"]);
        assert_eq!(a.environment_variables["GITHUB_RUN_ID"], "1");
    }

    #[test]
    fn cleanup_without_args_or_response_file() {
        let i = Input::read(
            &mut r#"{"command":"cleanup_job","responseFile":null,"state":{"job":"j"},"args":{}}"#
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(i.command, Command::CleanupJob);
        assert!(i.response_file.is_none());
    }

    #[test]
    fn response_shape_is_the_runners() {
        let r = PrepareJobResponse {
            state: serde_json::json!({"job": "01J"}),
            context: ResponseContext {
                container: Some(ResponseContainer {
                    id: "01J".into(),
                    network: String::new(),
                    ports: BTreeMap::new(),
                }),
                services: vec![],
            },
            is_alpine: false,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(
            j,
            r#"{"state":{"job":"01J"},"context":{"container":{"id":"01J","network":"","ports":{}},"services":[]},"isAlpine":false}"#
        );
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("r.json");
        write_response(Some(f.to_str().unwrap()), &r).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), j);
        write_response(None, &r).unwrap();
    }
}
