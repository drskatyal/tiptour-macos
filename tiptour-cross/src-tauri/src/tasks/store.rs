// File-backed task store. Personal-scale: a single JSON file with an
// atomic tmp+rename write. No SQLite or sled here — the task list will
// never exceed a few hundred entries per user.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use super::types::Task;

const ROOT_DIRECTORY_NAME: &str = "TipTour";
const TASKS_FILE_NAME: &str = "tasks.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct TasksFile {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    tasks: Vec<Task>,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

pub struct TaskStore {
    file_path: PathBuf,
    tasks: Vec<Task>,
}

impl TaskStore {
    pub fn open_default() -> Result<Self, String> {
        let base = dirs::data_local_dir().ok_or("no data_local_dir on this OS")?;
        let directory = base.join(ROOT_DIRECTORY_NAME);
        fs::create_dir_all(&directory).map_err(|e| format!("create tasks dir: {e}"))?;
        let file_path = directory.join(TASKS_FILE_NAME);
        let tasks = if file_path.exists() {
            let raw = fs::read_to_string(&file_path).map_err(|e| format!("read tasks: {e}"))?;
            let parsed: TasksFile =
                serde_json::from_str(&raw).map_err(|e| format!("parse tasks: {e}"))?;
            parsed.tasks
        } else {
            Vec::new()
        };
        Ok(Self { file_path, tasks })
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn tasks_mut(&mut self) -> &mut Vec<Task> {
        &mut self.tasks
    }

    pub fn persist(&self) -> Result<(), String> {
        let file = TasksFile {
            schema_version: CURRENT_SCHEMA_VERSION,
            tasks: self.tasks.clone(),
        };
        let serialized =
            serde_json::to_string_pretty(&file).map_err(|e| format!("serialize tasks: {e}"))?;
        let temporary_file_path = self.file_path.with_extension("json.tmp");
        {
            let mut handle = fs::File::create(&temporary_file_path)
                .map_err(|e| format!("create tmp tasks file: {e}"))?;
            handle
                .write_all(serialized.as_bytes())
                .map_err(|e| format!("write tmp tasks file: {e}"))?;
            handle
                .sync_all()
                .map_err(|e| format!("fsync tmp tasks file: {e}"))?;
        }
        fs::rename(&temporary_file_path, &self.file_path)
            .map_err(|e| format!("rename tmp tasks file: {e}"))?;
        Ok(())
    }
}
