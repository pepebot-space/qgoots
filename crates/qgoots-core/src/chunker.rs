use std::ops::Range;

#[derive(Debug, Clone)]
pub struct Chunk {
    pub content: String,
    pub start_pos: usize,
    pub end_pos: usize,
    pub token_count: usize,
}

// Very naive chunking by paragraphs/newlines as a starting point.
// Real implementation should parse markdown and respect the priority scores.
pub fn chunk_markdown(text: &str, target_size: usize, overlap: usize) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    
    // In a real implementation we would count tokens via tiktoken or similar.
    // Here we just approximate 1 token = 4 chars for simplicity
    let approx_token_len = |s: &str| s.len() / 4 + 1;

    let target_chars = target_size * 4;
    let overlap_chars = overlap * 4;
    
    let mut current_pos = 0;
    while current_pos < text.len() {
        let mut end_pos = (current_pos + target_chars).min(text.len());
        while end_pos < text.len() && !text.is_char_boundary(end_pos) {
            end_pos += 1;
        }
        
        // Try to find a logical break point (double newline)
        if end_pos < text.len() {
            let mut search_start = std::cmp::max(current_pos, end_pos.saturating_sub(100));
            while search_start > current_pos && !text.is_char_boundary(search_start) {
                search_start += 1;
            }
            let search_window = search_start..end_pos;
            let slice = &text[search_window];
            if let Some(break_idx) = slice.rfind("\n\n") {
                end_pos = search_start + break_idx + 2;
            }
        }
        
        let content = &text[current_pos..end_pos];
        let token_count = approx_token_len(content);
        
        chunks.push(Chunk {
            content: content.to_string(),
            start_pos: current_pos,
            end_pos,
            token_count,
        });
        
        if end_pos == text.len() {
            break;
        }
        
        // Move forward, keeping overlap
        current_pos = end_pos.saturating_sub(overlap_chars).max(current_pos + 1);
        while current_pos < text.len() && !text.is_char_boundary(current_pos) {
            current_pos += 1;
        }
    }
    
    chunks
}
