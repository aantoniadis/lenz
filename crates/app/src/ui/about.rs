//! GUI-only logo allocation. The RenderImage is owned only by the open dialog.
use super::*;
#[derive(Clone)]
pub(super) struct About {pub image:Arc<gpui::RenderImage>,pub scroll:usize}
impl About {
    pub fn open()->Self {
        // This small, checked-in PNG is trusted and tested; no filesystem reads.
        let pixels=image::load_from_memory(include_bytes!("../../../../assets/lenz-about.png"))
            .map_or_else(|_|image::RgbaImage::new(1,1),|image|image.into_rgba8());
        Self{image:lenz_gpui::image::texture(pixels),scroll:0}
    }
    /// Whether a point lies on the About box.
    pub fn contains(viewport:gpui::Size<Pixels>,at:gpui::Point<Pixels>)->bool{
        let (x,y,w,h)=layout(viewport);let (px_,py_)=(f32::from(at.x),f32::from(at.y));
        px_>=x&&px_<x+w&&py_>=y&&py_<y+h
    }
    pub fn close_bounds(viewport:gpui::Size<Pixels>)->Bounds<Pixels>{
        let (x,y,w,h)=layout(viewport);
        Bounds{origin:point(px(x+8.0),px((y+h-30.0).max(y))),size:size(px((w-16.0).max(0.0)),px(24.0_f32.min(h)))}
    }
    pub fn max_scroll(viewport:gpui::Size<Pixels>,char_w:f32)->usize{
        let (_,_,_,h)=layout(viewport);
        lines(viewport,char_w).len().saturating_sub(((h-82.0).max(0.0)/line_h())as usize)
    }
    #[allow(clippy::too_many_arguments)] // Palette plus adapter layout inputs.
    pub fn paint(&self, palette: &theme::Palette,bounds:Bounds<Pixels>,font:&Font,char_w:f32,window:&mut Window,cx:&mut App) {
        let (x,y,w,h)=layout(bounds.size);

        fill(window,x,y,w,h,palette.border);fill(window,x+1.0,y+1.0,(w-2.0).max(0.0),(h-2.0).max(0.0),palette.status_bg);
        let clip=Bounds{origin:point(px(x),px(y)),size:size(px(w),px(h))};
        window.with_content_mask(Some(ContentMask{bounds:clip}),|window|{
            text(window,cx,font,x+16.0,y+10.0,crate::about::TITLE,palette.accent);
            let logo=w>=600.0&&h>=190.0;
            let tx=x+if logo{132.0}else{16.0};
            if logo {let ib=Bounds{origin:point(px(x+16.0),px(y+48.0)),size:size(px(96.0),px(96.0))};window.paint_image(clip,ib,Default::default(),self.image.clone(),0,false).ok();}
            let top=y+40.0;let visible=((h-82.0).max(0.0)/line_h())as usize;
            let lines=lines(bounds.size,char_w);let start=self.scroll.min(lines.len().saturating_sub(visible));
            let content=Bounds{origin:point(px(tx),px(top)),size:size(px((x+w-tx-8.0).max(0.0)),px((h-82.0).max(0.0)))};
            window.with_content_mask(Some(ContentMask{bounds:content}),|window|{
                for (i,line) in lines.iter().skip(start).take(visible).enumerate(){text(window,cx,font,tx,top+i as f32*line_h(),line,palette.fg);}
            });
            let close=Self::close_bounds(bounds.size);let cy=f32::from(close.origin.y);
            fill(window,x+8.0,cy,(w-16.0).max(0.0),f32::from(close.size.height),palette.selected_bg);
            text(window,cx,font,x+16.0,cy,"Close · Esc / Enter     ↑/↓ scroll",palette.accent);
        });
    }
}
fn layout(viewport:gpui::Size<Pixels>)->(f32,f32,f32,f32){
    let vw=f32::from(viewport.width).max(0.0);let vh=f32::from(viewport.height).max(0.0);
    let w=(vw-16.0).clamp(0.0,760.0);let h=(vh-16.0).clamp(0.0,260.0);
    ((vw-w)/2.0,(vh-h)/2.0,w,h)
}

fn lines(viewport:gpui::Size<Pixels>,char_w:f32)->Vec<String>{
    let (_,_,w,h)=layout(viewport);
    let inset=if w>=600.0&&h>=190.0{132.0}else{16.0};
    let cols=((w-inset-8.0).max(0.0)/char_w.max(1.0)).floor().max(1.0)as usize;
    std::iter::once(crate::about::DESCRIPTION.to_owned()).chain(std::iter::once(String::new()))
        .chain(crate::about::ROWS.iter().map(|(k,v)|format!("{k}: {v}")))
        .flat_map(|line|{let chars=line.chars().collect::<Vec<_>>();if chars.is_empty(){vec![String::new()]}else{chars.chunks(cols).map(|chunk|chunk.iter().collect()).collect()}}).collect()
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn narrow_information_is_wrapped_without_losing_characters(){
        let viewport=size(px(300.0),px(150.0));let lines=lines(viewport,8.0);
        assert!(lines.iter().all(|line|line.chars().count()<=32));
        let joined=lines.concat();for (key,value)in crate::about::ROWS{assert!(joined.contains(&format!("{key}: {value}")));}
        assert!(About::max_scroll(viewport,8.0)>0);
    }
}
