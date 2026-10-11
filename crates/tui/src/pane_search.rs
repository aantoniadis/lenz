//! Shared terminal presentation for literal list searches.
use lenz::pane_search::PaneSearch;
use ratatui::{style::{Modifier,Style},text::{Line,Span}};

pub(crate) fn status(search:&PaneSearch,total:usize)->String {
    if search.searching(total)&&search.count()==0 {format!("Find: {}▏ · Searching…",search.query())}
    else {format!("{}{}",search.status(),if search.searching(total){" · searching…"}else{""})}
}
pub(crate) fn line(text:String,search:&PaneSearch,style:Style,palette:&lenz::theme::Palette)->Line<'static> {
    let text:String=text.chars().map(|c|if lenz::document::hides_text(c){'�'}else{c}).collect();
    let mut spans=Vec::new();let mut start=0;
    for range in search.ranges(&text) {
        if range.start>start {spans.push(Span::styled(text[start..range.start].to_string(),style));}
        spans.push(Span::styled(text[range.clone()].to_string(),style.bg(crate::chrome::color(palette.found_bg)).add_modifier(Modifier::BOLD)));
        start=range.end;
    }
    if start<text.len(){spans.push(Span::styled(text[start..].to_string(),style));}
    Line::from(spans)
}
