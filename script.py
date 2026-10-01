import re

with open('crates/savevault-db/src/repositories/backups.rs', 'r', encoding='utf-8') as f:
    content = f.read()

new_func = '''
    pub fn prune_old_records(&self, days_to_keep: u32) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days_to_keep as i64);
        let deleted = self.conn.execute(
            "DELETE FROM backups WHERE created_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(deleted)
    }
'''

content = re.sub(
    r'(pub fn find_by_id.*?Ok\(rows\.next\(\)\.transpose\(\)\?\)\n    })',
    r'\g<1>\n' + new_func,
    content,
    flags=re.DOTALL
)

with open('crates/savevault-db/src/repositories/backups.rs', 'w', encoding='utf-8') as f:
    f.write(content)


with open('crates/savevault-db/src/repositories/sync_queue.rs', 'r', encoding='utf-8') as f:
    content_sync = f.read()

new_func_sync = '''
    pub fn prune_completed_or_old(&self, days_to_keep: u32) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days_to_keep as i64);
        let deleted = self.conn.execute(
            "DELETE FROM sync_queue WHERE status = 'completed' OR created_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(deleted)
    }
'''

content_sync = re.sub(
    r'(pub fn update_status.*?Ok\(\(\)\)\n    })',
    r'\g<1>\n' + new_func_sync,
    content_sync,
    flags=re.DOTALL
)

with open('crates/savevault-db/src/repositories/sync_queue.rs', 'w', encoding='utf-8') as f:
    f.write(content_sync)
