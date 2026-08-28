use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

/// 日志轮转写入器
pub struct RotatingWriter {
    dir: PathBuf,
    base_name: String,
    max_size: u64,
    max_files: usize,
    current_file: Option<File>,
    current_size: u64,
}

impl RotatingWriter {
    /// 创建新的轮转写入器
    pub fn new(dir: PathBuf, base_name: &str, max_size: u64, max_files: usize) -> Self {
        let mut writer = Self {
            dir,
            base_name: base_name.to_string(),
            max_size,
            max_files,
            current_file: None,
            current_size: 0,
        };
        writer.open_current_file();
        writer
    }

    /// 打开当前日志文件
    fn open_current_file(&mut self) {
        let path = self.dir.join(&self.base_name);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();

        if let Some(f) = file {
            self.current_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            self.current_file = Some(f);
        }
    }

    /// 轮转日志文件
    fn rotate(&mut self) {
        // 关闭当前文件
        self.current_file = None;
        self.current_size = 0;

        // 删除最旧的文件（如果达到最大数量）
        for i in (1..self.max_files).rev() {
            let old_path = self.dir.join(format!("{}.{}", self.base_name, i));
            let new_path = self.dir.join(format!("{}.{}", self.base_name, i + 1));

            if old_path.exists() {
                if i + 1 >= self.max_files {
                    let _ = fs::remove_file(&old_path);
                } else {
                    let _ = fs::rename(&old_path, &new_path);
                }
            }
        }

        // 将当前文件重命名为 .1
        let current_path = self.dir.join(&self.base_name);
        let new_path = self.dir.join(format!("{}.1", self.base_name));
        if current_path.exists() {
            let _ = fs::rename(&current_path, &new_path);
        }

        // 创建新的当前文件
        self.open_current_file();
    }
}

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // 检查是否需要轮转
        if self.current_size + buf.len() as u64 > self.max_size {
            self.rotate();
        }

        // 写入数据
        if let Some(ref mut file) = self.current_file {
            let written = file.write(buf)?;
            self.current_size += written as u64;
            file.flush()?;
            Ok(written)
        } else {
            Err(io::Error::other("No log file open"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(ref mut file) = self.current_file {
            file.flush()?;
        }
        Ok(())
    }
}
