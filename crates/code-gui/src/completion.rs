//! Selected completion documentation is shaped off the input thread and fenced to its request.
use super::*;
use std::sync::{Weak, atomic::{AtomicBool, Ordering}};
use lenz_content::theme::Palette;
use lenz_gpui::markdown::ReadingStyle;

struct Request {
    preview: Arc<Preview>,
    width: f32,
    style: Arc<ReadingStyle>,
    palette: Arc<Palette>,
    tab: TabId,
    key: code_core::text::SnapshotKey,
    metadata: Weak<code_core::document::SourceMeta>,
    current: AtomicBool,
}

pub(super) struct Building {
    request: Arc<Request>,
    _task: gpui::Task<()>,
}
impl Drop for Building {
    fn drop(&mut self) {self.request.current.store(false,Ordering::Release);}
}

impl ReaderView {
    pub(super) fn completion_layout(&mut self, preview: Arc<Preview>, width: f32, window: &Window, cx: &mut Context<Self>) -> Option<Arc<preview::Layout>> {
        let style=self.reading_style(window);
        let state=self.session.state();
        let palette=state.palette().clone();
        let c=state.completion()?;
        let view=state.presentation_view(c.tab)?;
        let key=view.snapshot().key();
        let metadata=Arc::downgrade(view.snapshot().source_meta());
        let same=|layout:&preview::Layout|Arc::ptr_eq(&layout.preview,&preview)&&Arc::ptr_eq(&layout.style,&style)&&Arc::ptr_eq(&layout.palette,&palette);
        if self.completion_doc.as_ref().is_some_and(|d|!same(&d.layout)) {self.completion_doc=None;}
        let ready=self.completion_doc.as_ref().is_some_and(|d|same(&d.layout)&&d.layout.width==width);
        if ready {self.completion_doc_building=None;return self.completion_doc.as_ref().map(|d|d.layout.clone());}
        let building=self.completion_doc_building.as_ref().is_some_and(|b|{
            let r=&b.request;
            Arc::ptr_eq(&r.preview,&preview)&&r.width==width&&Arc::ptr_eq(&r.style,&style)&&Arc::ptr_eq(&r.palette,&palette)&&r.tab==c.tab&&r.key==key&&r.metadata.ptr_eq(&metadata)
        });
        if !ready && !building {
            self.completion_doc_building=None;
            let request=Arc::new(Request{preview,width,style,palette,tab:c.tab,key,metadata,current:AtomicBool::new(true)});
            let job=request.clone();
            let ts=gpui::WindowTextSystem::new(cx.text_system().clone());
            let task=cx.spawn(async move |this,cx|{
                let work=job.clone();
                let layout=cx.background_executor().spawn(async move {
                    preview::Layout::popup(work.preview.clone(),work.width,work.style.clone(),work.palette.clone(),&ts,&||work.current.load(Ordering::Acquire))
                }).await;
                let _=this.update(cx,|v,cx|{
                    if !v.completion_doc_building.as_ref().is_some_and(|b|Arc::ptr_eq(&b.request,&job)) {return;}
                    let state=v.session.state();
                    let current=job.current.load(Ordering::Acquire)
                        && state.completion().is_some_and(|c|c.tab==job.tab&&c.documentation_preview().is_some_and(|p|Arc::ptr_eq(p,&job.preview)))
                        && state.presentation_view(job.tab).is_some_and(|view|view.snapshot().key()==job.key&&job.metadata.ptr_eq(&Arc::downgrade(view.snapshot().source_meta())));
                    v.completion_doc_building=None;
                    if current {
                        let top=v.completion_doc.as_ref().map_or(0.0,|d|d.top);
                        v.completion_doc=Some(DocPopup{layout:Arc::new(layout),top,bounds:Bounds::default()});
                    }
                    cx.notify();
                });
            });
            self.completion_doc_building=Some(Building{request,_task:task});
        }
        self.completion_doc.as_ref().map(|d|d.layout.clone())
    }
}
