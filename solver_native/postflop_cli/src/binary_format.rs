use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::export::StrategyData;

pub const MAGIC: &[u8; 8] = b"RTA_MTX1";
pub const HEADER_SIZE: usize = 128;
pub const INDEX_ENTRY_SIZE: usize = 80;
pub const FOOTER_SIZE: usize = 36;

const VERSION: u32 = 1;

pub fn write_file(
    data: &[StrategyData],
    tree_version: &str,
    range_premise_version: &str,
    exploitability: f32,
    iterations: u32,
    path: &Path,
) -> Result<(), String> {
    let num_nodes = data.len() as u32;
    let mut buf: Vec<u8> = Vec::new();

    let data_offset = HEADER_SIZE as u64 + num_nodes as u64 * INDEX_ENTRY_SIZE as u64;

    let mut body: Vec<u8> = Vec::new();
    let mut index_entries: Vec<IndexEntry> = Vec::with_capacity(num_nodes as usize);

    for node in data {
        let strategy_offset = body.len() as u32;
        let strategy_bytes =
            unsafe { std::slice::from_raw_parts(node.strategy.as_ptr() as *const u8, node.strategy.len() * 4) };
        body.extend_from_slice(strategy_bytes);

        let ev_offset = body.len() as u32;
        let ev_bytes =
            unsafe { std::slice::from_raw_parts(node.ev.as_ptr() as *const u8, node.ev.len() * 4) };
        body.extend_from_slice(ev_bytes);

        index_entries.push(build_index_entry(node, strategy_offset, ev_offset));
    }

    let body_len = body.len();
    let total_size = HEADER_SIZE + index_entries.len() * INDEX_ENTRY_SIZE + body_len + FOOTER_SIZE;

    buf.resize(HEADER_SIZE, 0);
    buf[0..8].copy_from_slice(MAGIC);
    write_u32_le(&mut buf, 8, VERSION);
    write_u32_le(&mut buf, 12, num_nodes);
    write_u64_le(&mut buf, 16, data_offset);
    write_padded_str(&mut buf, 24, 24, tree_version);
    write_padded_str(&mut buf, 48, 40, range_premise_version);
    write_u32_le(&mut buf, 104, total_size as u32);
    buf[108] = 0;
    buf[109] = 0;
    write_f32_le(&mut buf, 110, exploitability);
    write_u32_le(&mut buf, 114, iterations);

    for entry in &index_entries {
        let mut entry_bytes = vec![0u8; INDEX_ENTRY_SIZE];
        write_index_entry(&mut entry_bytes, entry);
        buf.extend_from_slice(&entry_bytes);
    }

    buf.extend_from_slice(&body);

    let mut hasher = Sha256::new();
    hasher.update(&buf);
    let checksum = hasher.finalize();
    buf.extend_from_slice(&checksum);

    let total_size_bytes = (total_size as u32).to_le_bytes();
    buf.extend_from_slice(&total_size_bytes);

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, &buf).map_err(|e| format!("write error: {e}"))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("rename error: {e}"))?;

    Ok(())
}

struct IndexEntry {
    node_path: [u16; 32],
    node_path_len: u8,
    player: u8,
    board_state: u8,
    num_actions: u16,
    num_hands: u16,
    strategy_offset: u32,
    ev_offset: u32,
}

fn build_index_entry(node: &StrategyData, strategy_offset: u32, ev_offset: u32) -> IndexEntry {
    let mut node_path = [0xFFFFu16; 32];
    let len = node.node_path.len().min(32);
    for (i, &step) in node.node_path.iter().enumerate().take(len) {
        node_path[i] = step as u16;
    }
    IndexEntry {
        node_path,
        node_path_len: len as u8,
        player: node.player as u8,
        board_state: node.board_state,
        num_actions: node.actions.len() as u16,
        num_hands: node.num_hands as u16,
        strategy_offset,
        ev_offset,
    }
}

fn write_index_entry(buf: &mut [u8], entry: &IndexEntry) {
    for (i, &v) in entry.node_path.iter().enumerate() {
        buf[i * 2] = v as u8;
        buf[i * 2 + 1] = (v >> 8) as u8;
    }
    buf[64] = entry.node_path_len;
    buf[65] = entry.player;
    buf[66] = entry.board_state;
    buf[67] = 0;
    buf[68] = entry.num_actions as u8;
    buf[69] = (entry.num_actions >> 8) as u8;
    buf[70] = entry.num_hands as u8;
    buf[71] = (entry.num_hands >> 8) as u8;
    write_u32_at(buf, 72, entry.strategy_offset);
    write_u32_at(buf, 76, entry.ev_offset);
}

fn write_u32_le(buf: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}

fn write_u64_le(buf: &mut [u8], offset: usize, value: u64) {
    let bytes = value.to_le_bytes();
    buf[offset..offset + 8].copy_from_slice(&bytes);
}

fn write_padded_str(buf: &mut [u8], offset: usize, max_len: usize, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(max_len);
    buf[offset..offset + len].copy_from_slice(&bytes[..len]);
}

fn write_u32_at(buf: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}

fn write_f32_le(buf: &mut [u8], offset: usize, value: f32) {
    let bytes = value.to_le_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}
