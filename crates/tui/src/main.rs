#![forbid(unsafe_code)]
/// Freed blocks of 1 MiB and up leave the process at once instead of staying in libmalloc's cache.
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOCATOR: lenz::large_blocks::LargeBlocks = lenz::large_blocks::LargeBlocks;
#[cfg(test)]
mod theme_tests;
mod themes;
mod events;
mod help;
mod about;
mod about_sixel;
mod chrome;
mod icons;
mod files;
mod summary;
mod commit_panel;
mod pane_search;
mod panes;
#[cfg(test)]
mod tab_perf_tests;
#[cfg(test)]
mod parity_tests;
mod search;
mod clipboard;
mod merge;
mod repository;
mod pr_details;
mod visual;
mod app;
mod context;
mod palette;
mod md_images;
mod render;
mod markdown;
mod repo_picker;
#[cfg(test)]
#[path = "../../../benches/support.rs"]
mod perf;
#[cfg(test)]
use lenz::memory_probe;

use std::{io::{self, IsTerminal}, process::ExitCode, time::{Duration,Instant}};
use crossterm::{event::{self, EnableMouseCapture, DisableMouseCapture, EnableBracketedPaste, DisableBracketedPaste, EnableFocusChange, DisableFocusChange}, execute};
use lenz::cli::{self, Launch, Request};

const HELP: &str = "Lenz terminal
usage: lenz-tui [options] [--staged] [<rev> [<rev>]] [-- <path>...]
       lenz-tui [options] [--no-index] <left> <right>
       lenz-tui log [<rev>...] [-- <path>...] | log --follow <file>
       lenz-tui pr [<number>|<url>]
       lenz-tui merge [[--] <path>...] | merge <base> <local> <remote> <merged>

Same comparison options as lenz, including --blame and --line N.
Optional icons: LENZ_ICONS=nerd (Nerd Font Mono); View menu toggles icons.
n/p changes; [/] files; / find; f files; : commands; Ctrl-R repository;
A applied history; Ctrl-Z/Ctrl-Y undo/redo drafts; Ctrl-S saves;
Alt-Shift-arrows apply to draft; commands discard unsaved/revert hunk; b blame; ? help.
lenz-tui report --last [--print] retrieves the last error and GitHub issue draft.";

fn request(args:&[String],cwd:&std::path::Path)->Result<Request,String>{cli::parse(args,cwd).map_err(|e|e.0.replace("lenz","lenz-tui"))}

enum Screen { Diff(Box<app::App>), Merge{controller:Box<merge::Merger>,parent:Option<usize>} }
struct Screens {themes:lenz::theme::ThemeService,chooser:Option<themes::Chooser>, views:Vec<Screen>, active:usize, exit_code:Option<u8>, /** Pixel artwork via kitty graphics (kitty, Ghostty). */ kitty:bool, about_graphics:Option<(md_images::Picture,md_images::Images)>, sixel:Option<bool>, about_sixel:Option<about_sixel::Logo>, cell_pixels:Option<crossterm::terminal::WindowSize>, sixel_repaint:bool, /** Image deletes of closed views, written with the next frame. */ backlog:Vec<String>, #[cfg(test)] _theme_scratch:Option<tempfile::TempDir> }
impl Screens {
    fn open(request:Request,cwd:std::path::PathBuf)->Self{
        #[cfg(test)] {
            let scratch=tempfile::tempdir().unwrap();let paths=lenz::theme::ThemePaths::under(&scratch.path().join("config"),&scratch.path().join("state"),"lenz");
            let mut screens=Self::open_with_theme(request,cwd,lenz::theme::ThemeService::new_deferred(paths,lenz::theme::overrides_from_env(),true));screens._theme_scratch=Some(scratch);screens
        }
        #[cfg(not(test))] {let mut screens=Self::open_with_theme(request,cwd,lenz::theme::ThemeService::new_deferred(lenz::theme::paths_from_env(),lenz::theme::overrides_from_env(),true));screens.kitty=md_images::kitty_supported(|name|std::env::var(name).ok());screens}
    }
    fn open_with_theme(request:Request,cwd:std::path::PathBuf,mut themes:lenz::theme::ThemeService)->Self{
        // one notice line: settings problems first
        let mut notices=themes.take_notices();if let Some(settings)=lenz::settings::take_notice(){notices.insert(0,settings);}
        let mut view=if matches!(request.launch,Launch::Merge(_)){Screen::Merge{controller:Box::new(merge::Merger::open(request,cwd)),parent:None}}else{Screen::Diff(Box::new(app::App::open(request,cwd)))};
        if let Some(n)=notices.into_iter().next(){match &mut view{Screen::Diff(a)=>a.session.set_notice(n),Screen::Merge{controller,..}=>controller.workspace.notice=Some(n)}}
        Self{themes,chooser:None,views:vec![view],active:0,exit_code:None,kitty:false,about_graphics:None,sixel:None,about_sixel:None,cell_pixels:None,sixel_repaint:false,backlog:Vec::new(),#[cfg(test)]_theme_scratch:None}
    }
    fn remove(&mut self,index:usize){if let Some(Screen::Diff(app))=self.views.get_mut(index){app.markdown.images.clear();self.backlog.extend(app.markdown.images.take_graphics());}self.views.remove(index);for view in &mut self.views{if let Screen::Merge{parent:Some(parent),..}=view{if *parent==index{if let Screen::Merge{parent,..}=view{*parent=None;}}else if *parent>index{*parent-=1;}}}self.active=self.active.min(self.views.len().saturating_sub(1));}
    fn refresh_parent(&mut self,index:usize){if let Some(Screen::Diff(app))=self.views.get_mut(index){app.session.reload_set();if let Some(repo)=&mut app.repository{repo.core.execute(lenz::repository_session::RepositoryCommand::RefreshRepository);if !repo.core.picker.shown{repo.core.execute(lenz::repository_session::RepositoryCommand::TogglePicker);}}}}
    fn poll(&mut self)->bool {
        let mut changed=false;
        while let Ok(update)=self.themes.updates().try_recv(){changed|=self.theme_update(update);}
        let mut requested=false;let mut additions=Vec::new();let mut backs=Vec::new();
        for (index,view) in self.views.iter_mut().enumerate(){
            match view {
                Screen::Diff(app)=>{if self.kitty{app.markdown.images.enable_kitty();}let update=app.poll();requested|=std::mem::take(&mut app.theme_request);changed|=update&&index==self.active;if let Some((req,cwd))=app.new_view.take(){additions.push(Screen::Diff(Box::new(app::App::open(req,cwd))));}
        if let Some(root)=app.conflicts.take(){additions.push(Screen::Merge{controller:Box::new(merge::Merger::from_repository(root,Vec::new())),parent:Some(index)});}},
                Screen::Merge{controller,parent}=>{let update=controller.poll();requested|=std::mem::take(&mut controller.theme_request);changed|=update&&index==self.active;if let Some((req,cwd))=controller.new_view.take(){additions.push(Screen::Diff(Box::new(app::App::open(req,cwd))));}
        if controller.back{backs.push((index,*parent,controller.workspace.root.clone()));}}
            }
        }
        if requested{self.open_themes();changed=true;}
        for (index,parent,root) in backs.into_iter().rev(){
            if let Some(parent)=parent{self.refresh_parent(parent);self.active=parent;self.remove(index);self.activate();}
            else if let Some(root)=root&&let Ok(req)=cli::parse(&["log".into()],&root){self.views[index]=Screen::Diff(Box::new(app::App::open(req,root)));}
            changed=true;
        }
        for view in additions{self.views.push(view);self.active=self.views.len()-1;changed=true;}
        if let Some(view)=self.views.get(self.active){let code=match view{Screen::Diff(a)=>a.quit.then(||if a.failed(){2}else{0}),Screen::Merge{controller,..}=>controller.quit.then(||controller.exit_code())};if let Some(code)=code{if self.views.len()==1{self.exit_code=Some(code);}else{let index=self.active;if let Screen::Merge{parent:Some(parent),..}=&self.views[index]{let parent=*parent;self.refresh_parent(parent);self.active=parent;}self.remove(index);self.activate();changed=true;}}}
        changed
    }
    fn activate(&mut self){match &mut self.views[self.active]{Screen::Diff(app)=>{if let Some(repo)=&mut app.repository{repo.handle(event::Event::FocusGained);}},Screen::Merge{controller,..}=>{controller.handle(event::Event::FocusGained);}}}
    fn busy(&self)->bool{self.views.iter().any(|v|match v{Screen::Diff(a)=>a.busy(),Screen::Merge{controller,..}=>controller.busy()})}
    fn notice(&mut self,n:String){if let Some(view)=self.views.get_mut(self.active){match view{Screen::Diff(a)=>a.session.set_notice(n),Screen::Merge{controller,..}=>controller.workspace.notice=Some(n)}}}
    fn theme_update(&mut self,update:lenz::theme::ThemeUpdate)->bool{let effect=self.themes.apply(update);let dirty=effect.repaint||effect.notice.is_some();if let Some(n)=effect.notice{self.notice(n);}dirty}
    fn cancel_pane_drags(&mut self){for view in &mut self.views {match view {Screen::Diff(a)=>a.cancel_pane_drag(),Screen::Merge{controller,..}=>controller.cancel_pane_drag()}}}
    fn open_themes(&mut self){self.cancel_pane_drags();self.themes.execute(lenz::theme::ThemeCommand::OpenChooser);self.chooser=Some(themes::Chooser::new(&self.themes));}
    fn handle(&mut self,event:event::Event)->bool{
        if matches!(event,event::Event::Resize(..)) && self.about_sixel.take().is_some(){self.sixel_repaint=true;}
        let quit=matches!(event,event::Event::Key(k)if k.kind!=event::KeyEventKind::Release&&k.modifiers.contains(event::KeyModifiers::CONTROL)&&!k.modifiers.contains(event::KeyModifiers::SHIFT)&&matches!(k.code,event::KeyCode::Char('c'|'q')));
        if quit {self.chooser=None;}
        if let Some(chooser)=&mut self.chooser{match chooser.handle(&event){themes::Action::Close=>self.chooser=None,themes::Action::Select(selection)=>{let effect=self.themes.execute(lenz::theme::ThemeCommand::Select(selection));if let Some(n)=effect.notice{self.notice(n);}self.chooser=None;},themes::Action::None=>{}}return true;}
        if let event::Event::Key(key)=event&&key.kind!=event::KeyEventKind::Release&&key.code==event::KeyCode::F(8){self.cancel_pane_drags();let len=self.views.len();self.active=if key.modifiers.contains(event::KeyModifiers::SHIFT){(self.active+len-1)%len}else{(self.active+1)%len};self.activate();return true;}
        let (handled,requested)=match &mut self.views[self.active]{Screen::Diff(a)=>(a.handle(event),std::mem::take(&mut a.theme_request)),Screen::Merge{controller,..}=>{let handled=controller.handle(event);let requested=controller.take_theme_request();(handled,requested)}};
        if requested{self.open_themes();}handled
    }
    fn render(&mut self,frame:&mut ratatui::Frame){let mut snapshot=self.themes.snapshot();
        // the picker shows the highlighted theme live; a distinct revision refreshes cached drawing
        if let Some((selected,palette))=self.chooser.as_ref().and_then(|c|c.highlighted().and_then(|id|self.themes.preview(id)).map(|p|(c.selected(),p))){snapshot.palette=palette;snapshot.revision=u64::MAX-selected as u64;}
        let palette=&snapshot.palette;match &mut self.views[self.active]{Screen::Diff(a)=>{a.palette=palette.clone();a.theme_revision=snapshot.revision;a.render(frame)},Screen::Merge{controller,..}=>{controller.palette=palette.clone();controller.theme_revision=snapshot.revision;controller.render(frame)}}
        if self.views.len()>1{let title=format!(" Lenz · view {}/{} · F8/Shift-F8 switch · Ctrl-Q close view",self.active+1,self.views.len());let area=frame.area();frame.render_widget(ratatui::widgets::Paragraph::new(title).style(chrome::style_with(palette).fg(chrome::color(palette.accent))),ratatui::layout::Rect::new(area.x,area.bottom().saturating_sub(1),area.width,1));}
        if let Some(chooser)=&mut self.chooser{chooser.draw(frame,&self.themes,palette);}
        let logo=if self.chooser.is_some(){ratatui::layout::Rect::default()}else{match &self.views[self.active]{Screen::Diff(app)=>app.about.as_ref().map_or(ratatui::layout::Rect::default(),|about|about.logo_rect),Screen::Merge{controller,..}=>controller.about_logo()}};
        if self.kitty && !logo.is_empty() {
            if self.about_graphics.is_none() {
                // The same small PNG as the GUI, decoded only while a visible dialog needs it.
                if let Ok(decoded)=lenz::images::decode(include_bytes!("../../../assets/lenz-about.png"),lenz::images::Format::Png) {
                    let mut images=md_images::Images::default();images.enable_kitty();
                    self.about_graphics=Some((std::sync::Arc::new(decoded.pixels),images));
                }
            }
            if let Some((picture,images))=&mut self.about_graphics {
                images.keep([(0,0,&*picture,logo.width,logo.height)].into_iter());let id=images.show(0,0,picture,logo.width,logo.height);
                frame.buffer_mut().set_style(logo,ratatui::style::Style::default().bg(chrome::color(palette.status_bg)));
                for row in 0..logo.height{md_images::draw_row(frame.buffer_mut(),logo,logo.x,logo.y+row,row,logo.height,logo.width,picture,palette.status_bg,id);}
            }
        } else {self.clear_about_graphics();}
        if !self.kitty && !logo.is_empty() && *self.sixel.get_or_insert_with(||about_sixel::supported(|name|std::env::var(name).ok(),about_sixel::foot_launcher)) {
            if self.cell_pixels.is_none(){self.cell_pixels=crossterm::terminal::window_size().ok();}
            if let Some(size)=&self.cell_pixels {
                if !self.about_sixel.as_ref().is_some_and(|image|image.matches(logo,size,palette.status_bg)) {
                    self.sixel_repaint|=self.about_sixel.is_some();
                    self.about_sixel=about_sixel::Logo::new(logo,size,palette.status_bg);
                    if let Some(image)=&self.about_sixel{self.backlog.push(image.sequence.clone());}
                }
                if self.about_sixel.is_some(){frame.render_widget(ratatui::widgets::Clear,logo);frame.render_widget(ratatui::widgets::Block::default().style(ratatui::style::Style::default().bg(chrome::color(palette.status_bg))),logo);}
            }
        } else if self.about_sixel.take().is_some(){self.sixel_repaint=true;}
    }
    fn clear_about_graphics(&mut self){if let Some((_,mut images))=self.about_graphics.take(){images.clear();self.backlog.extend(images.take_graphics());}}
    /// Terminal image uploads and Kitty deletes, written after a frame.
    fn take_graphics(&mut self)->Vec<String>{std::mem::take(&mut self.backlog).into_iter().chain(self.about_graphics.as_mut().into_iter().flat_map(|(_,images)|images.take_graphics())).chain(self.views.iter_mut().flat_map(|view|match view{Screen::Diff(app)=>app.markdown.images.take_graphics(),Screen::Merge{..}=>Vec::new()})).collect()}
    /// At exit: every view's images leave the terminal.
    fn shutdown_graphics(&mut self)->Vec<String>{self.about_sixel=None;self.clear_about_graphics();for view in &mut self.views{if let Screen::Diff(app)=view{app.markdown.images.clear();}}self.take_graphics()}
    fn exit(&self)->Option<u8>{self.exit_code}
}

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        restore_input_modes();
        ratatui::restore();
    }
}

fn restore_input_modes() {
    let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste, DisableFocusChange);
}

/// Outside a repository: the repository picker, before any comparison opens. `None` when the user quits.
fn choose_repository(terminal:&mut ratatui::DefaultTerminal,cwd:std::path::PathBuf)->io::Result<Option<std::path::PathBuf>>{
    let themes=lenz::theme::ThemeService::new_deferred(lenz::theme::paths_from_env(),lenz::theme::overrides_from_env(),true);
    let palette=themes.snapshot().palette;
    let mut view=repo_picker::View::new(lenz::repo_picker::RepoPicker::new(cwd));
    loop {
        terminal.draw(|frame|view.draw(frame,&palette))?;
        match view.handle(&event::read()?) {
            repo_picker::Outcome::Quit=>return Ok(None),
            repo_picker::Outcome::Chosen(root)=>return Ok(Some(root)),
            repo_picker::Outcome::None=>{}
        }
    }
}

fn init_terminal() -> io::Result<ratatui::DefaultTerminal> {
    let terminal = ratatui::try_init()?;
    // Destructors do not run with panic=abort. Clear our extra terminal modes,
    // then let Ratatui restore raw/alternate-screen state before diagnostics.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| { restore_input_modes(); previous(info); }));
    execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste, EnableFocusChange)?;
    Ok(terminal)
}

fn run(args: &[String], mut request: Request, mut cwd: std::path::PathBuf) -> io::Result<u8> {
    let _restore = Restore;
    let mut terminal = init_terminal()?;
    if lenz::repo_picker::needed(&request,&cwd) {
        let Some(root)=choose_repository(&mut terminal,cwd)? else { return Ok(0) };
        std::env::set_current_dir(&root)?;
        request=self::request(args,&root).map_err(io::Error::other)?;
        cwd=root;
    }
    let mut app = Screens::open(request,cwd);
    // Install terminal input after the first frame; bytes stay buffered meanwhile.
    let mut input=None;
    let updates=app.themes.updates();
    let mut dirty = true;
    let mut fast_poll_until=Instant::now()+Duration::from_millis(100);
    loop {
        dirty |= app.poll();
        if dirty { terminal.draw(|frame| app.render(frame))?;
            // Sixel has no image-delete command: erase terminal pixels and repaint underlying cells.
            if std::mem::take(&mut app.sixel_repaint){
                use ratatui::backend::Backend;
                // Cursor is hidden; a direct clear avoids Terminal::clear's cursor-query round trip.
                terminal.backend_mut().clear()?;terminal.swap_buffers();
                terminal.draw(|frame|app.render(frame))?;
            }
            {let graphics=app.take_graphics();if !graphics.is_empty(){use std::io::Write;let mut out=std::io::stdout().lock();for sequence in graphics{out.write_all(sequence.as_bytes())?;}out.flush()?;}} app.themes.start();for notice in app.themes.take_notices(){app.notice(notice);} dirty = app.poll(); }
        if dirty { continue; }
        if let Some(code)=app.exit(){let graphics=app.shutdown_graphics();if !graphics.is_empty(){use std::io::Write;let mut out=std::io::stdout().lock();for sequence in graphics{out.write_all(sequence.as_bytes())?;}out.flush()?;}return Ok(code);}
        // No periodic redraw while idle. During background work only the inbox is polled.
        let delay=app.busy().then(||Duration::from_millis(if Instant::now()<fast_poll_until{1}else{16}));
        match futures_lite::future::block_on(events::wait(input.get_or_insert_with(event::EventStream::new),&updates,delay)){
            events::Wake::Input(event)=>{let event=event?;if matches!(event,event::Event::Resize(..)){app.cell_pixels=None;}dirty|=app.handle(event);},
            events::Wake::Theme(update)=>dirty|=app.theme_update(update),
            events::Wake::Work=>continue,
            events::Wake::ThemesClosed=>{app.notice("Live theme updates unavailable".into());dirty=true;},
            events::Wake::Closed=>return Err(io::Error::new(io::ErrorKind::UnexpectedEof,"Terminal/theme event stream closed")),
        }
        fast_poll_until=Instant::now()+Duration::from_millis(100);
        // Render may expose new blocks after a resize or scroll; poll schedules their refinement.
    }
}

fn main() -> ExitCode {
    lenz::diagnostics::install("TUI");
    let (settings, notices) = lenz::settings::load();
    lenz::settings::init(settings);
    lenz::settings::stash_notices(notices);
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Some(code) = lenz::diagnostics::report_cli(&args) { return code; }
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] { println!("{HELP}"); return ExitCode::SUCCESS; }
    let result = (|| {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let request = request(&args, &cwd)?;
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err("lenz-tui needs an interactive terminal on stdin and stdout".into());
        }
        run(&args, request, cwd).map_err(|e| { lenz::diagnostics::failure("Terminal input/output", &e.to_string(), "Check that the terminal is available, then restart lenz-tui."); format!("lenz-tui: {e}") })
    })();
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => { eprintln!("{e}"); ExitCode::from(2) }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test]
    fn about_sixel_redraws_resizes_and_releases_hidden_artwork() {
        let dir=tempfile::tempdir().unwrap();
        let request=||cli::parse(&["--about".into()],dir.path()).unwrap();
        let mut screens=Screens::open(request(),dir.path().into());
        screens.sixel=Some(true);
        screens.cell_pixels=Some(crossterm::terminal::WindowSize{columns:100,rows:30,width:900,height:540});
        let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(100,30)).unwrap();
        terminal.draw(|f|screens.render(f)).unwrap();
        assert!(screens.take_graphics().concat().contains("q\"1;1;216;216"));
        let logo=screens.about_sixel.as_ref().unwrap().rect;
        assert!(terminal.backend().buffer().content.iter().all(|c|c.symbol()!="▀"));
        terminal.draw(|f|screens.render(f)).unwrap();
        assert!(screens.take_graphics().is_empty(),"idle rendering reuses the cached image");
        screens.handle(event::Event::Resize(100,30));
        terminal.draw(|f|screens.render(f)).unwrap();
        assert!(screens.take_graphics().concat().contains("\x1bP"),"font changes repaint even at unchanged cell dimensions");
        assert!(std::mem::take(&mut screens.sixel_repaint));
        terminal.backend_mut().resize(30,10);
        terminal.draw(|f|screens.render(f)).unwrap();
        assert!(screens.about_sixel.is_none());assert!(std::mem::take(&mut screens.sixel_repaint));
        terminal.backend_mut().resize(100,30);
        terminal.draw(|f|screens.render(f)).unwrap();screens.take_graphics();
        assert_eq!(screens.about_sixel.as_ref().unwrap().rect,logo);
        screens.open_themes();terminal.draw(|f|screens.render(f)).unwrap();
        assert!(screens.about_sixel.is_none(),"theme chooser covers and releases the image");
        screens.chooser=None;terminal.draw(|f|screens.render(f)).unwrap();screens.take_graphics();
        screens.handle(event::Event::Key(event::KeyEvent::new(event::KeyCode::Esc,event::KeyModifiers::NONE)));
        terminal.draw(|f|screens.render(f)).unwrap();
        assert!(screens.about_sixel.is_none());assert!(std::mem::take(&mut screens.sixel_repaint));
        assert!(screens.take_graphics().is_empty(),"Sixel does not send Kitty deletes or uploads");
    }
    #[test]
    fn about_uses_pixels_in_graphics_terminals_and_releases_hidden_logos() {
        let dir=tempfile::tempdir().unwrap();
        let request=||cli::parse(&["--about".into()],dir.path()).unwrap();
        let mut screens=Screens::open(request(),dir.path().into());
        let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(100,30)).unwrap();
        terminal.draw(|frame|screens.render(frame)).unwrap();
        assert!(screens.take_graphics().is_empty(),"cell fallback needs no graphics");
        screens.kitty=true;
        terminal.draw(|frame|screens.render(frame)).unwrap();
        let upload=screens.take_graphics().concat();
        assert!(upload.contains("f=32,s=96,v=96"),"transmit the GUI's detailed logo: {upload}");
        assert!(upload.contains("c=24,r=12"));
        assert_eq!(terminal.backend().buffer().content.iter().filter(|c|c.symbol().starts_with('\u{10EEEE}')).count(),288);
        let background=chrome::color(screens.themes.snapshot().palette.status_bg);
        assert!(terminal.backend().buffer().content.iter().filter(|c|c.symbol().starts_with('\u{10EEEE}')).all(|c|c.bg==background),"transparent PNG edges must not reveal the cell fallback underneath");
        let logo=std::sync::Arc::downgrade(&screens.about_graphics.as_ref().unwrap().0);
        terminal.draw(|frame|screens.render(frame)).unwrap();
        assert!(screens.take_graphics().is_empty(),"ordinary redraw must reuse the image");
        terminal.backend_mut().resize(30,10);
        terminal.draw(|frame|screens.render(frame)).unwrap();
        assert!(screens.take_graphics().concat().contains("a=d,d=I"),"narrow layout frees terminal artwork");
        assert!(logo.upgrade().is_none(),"narrow layout releases CPU artwork too");
        terminal.backend_mut().resize(100,30);
        terminal.draw(|frame|screens.render(frame)).unwrap();
        assert!(screens.take_graphics().concat().contains("a=t"));
        screens.handle(event::Event::Key(event::KeyEvent::new(event::KeyCode::Esc,event::KeyModifiers::NONE)));
        terminal.draw(|frame|screens.render(frame)).unwrap();
        assert!(screens.take_graphics().concat().contains("a=d,d=I"),"dismissal frees terminal artwork");
        assert!(!terminal.backend().buffer().content.iter().any(|c|c.symbol().starts_with('\u{10EEEE}')));
        screens.views.push(Screen::Diff(Box::new(app::App::open(request(),dir.path().into()))));
        screens.active=1;
        terminal.draw(|frame|screens.render(frame)).unwrap();
        assert!(screens.take_graphics().concat().contains("a=t"));
        screens.active=0;
        terminal.draw(|frame|screens.render(frame)).unwrap();
        assert!(screens.take_graphics().concat().contains("a=d,d=I"),"switching views hides and frees the logo");
        screens.active=1;
        terminal.draw(|frame|screens.render(frame)).unwrap();
        screens.take_graphics();
        assert!(screens.shutdown_graphics().concat().contains("a=d,d=I"),"exit frees the logo");
    }
    #[test]
    fn about_pixel_artwork_is_used_by_merge_and_nested_comparison_menus() {
        for kitty in [true,false] {
        let dir=tempfile::tempdir().unwrap();
        for (name,bytes) in [("base",include_bytes!("../../../assets/lenz-about.png").as_slice()),("ours",include_bytes!("../../../assets/lenz-icon.png").as_slice()),("theirs",include_bytes!("../../../assets/lenz-icon-source.png").as_slice())]{std::fs::write(dir.path().join(name),bytes).unwrap();}
        std::fs::write(dir.path().join("result"),include_bytes!("../../../assets/lenz-icon.png")).unwrap();
        let request=cli::parse(&["merge","base","ours","theirs","result"].map(str::to_owned),dir.path()).unwrap();
        let mut screens=Screens::open(request,dir.path().into());screens.kitty=kitty;screens.sixel=Some(!kitty);screens.cell_pixels=Some(crossterm::terminal::WindowSize{columns:100,rows:30,width:900,height:540});
        for _ in 0..10000{screens.poll();if !screens.busy(){break;}std::thread::sleep(Duration::from_millis(1));}
        assert!(!screens.busy());
        let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(100,30)).unwrap();
        let key=|code|event::Event::Key(event::KeyEvent::new(code,event::KeyModifiers::NONE));
        for menu in [event::KeyCode::Char(':'),event::KeyCode::F(4)] {
            screens.handle(key(menu));for c in "about".chars(){screens.handle(key(event::KeyCode::Char(c)));}screens.handle(key(event::KeyCode::Enter));
            terminal.draw(|frame|screens.render(frame)).unwrap();
            let upload=screens.take_graphics().concat();
            assert!(upload.contains(if kitty{"f=32,s=96,v=96"}else{"\x1bP0;1;0q"}),"merge menu {menu:?}, kitty={kitty}");
            screens.handle(key(event::KeyCode::Char('2')));screens.handle(event::Event::Key(event::KeyEvent::new(event::KeyCode::Char('s'),event::KeyModifiers::CONTROL)));
            assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),include_bytes!("../../../assets/lenz-icon.png"));
            screens.handle(key(event::KeyCode::Esc));terminal.draw(|frame|screens.render(frame)).unwrap();
            if kitty{assert!(screens.take_graphics().concat().contains("a=d,d=I"));}else{assert!(screens.about_sixel.is_none());assert!(std::mem::take(&mut screens.sixel_repaint));}
        }
        }
    }
    #[test]
    fn terminal_panic_child() {
        if std::env::var_os("LENZ_TEST_TERMINAL_PANIC").is_none() { return; }
        lenz::diagnostics::install("TUI");
        let _terminal = init_terminal().unwrap();
        lenz::diagnostics::breadcrumb("Render comparison");
        panic!("terminal render failure");
    }
    #[test] fn supported_launches_parse_before_terminal_setup(){for args in [vec!["log"],vec!["pr"],vec!["merge"],vec!["--blame"]]{let args=args.into_iter().map(str::to_string).collect::<Vec<_>>();assert!(request(&args,std::path::Path::new("/tmp")).is_ok());}}
    #[test] fn independent_views_preserve_state_and_close_only_active_view(){
        let dir=tempfile::tempdir().unwrap();std::fs::write(dir.path().join("left"),"old words ".repeat(100)).unwrap();std::fs::write(dir.path().join("right"),"new words ".repeat(100)).unwrap();
        let req=cli::parse(&["--no-index".into(),"left".into(),"right".into()],dir.path()).unwrap();let mut screens=Screens::open(cli::parse(&["--no-index".into(),"left".into(),"right".into()],dir.path()).unwrap(),dir.path().into());
        // Save a valid position after the initial document has loaded.
        for _ in 0..1000{screens.poll();if matches!(&screens.views[0],Screen::Diff(app)if !app.busy()){break;}std::thread::sleep(std::time::Duration::from_millis(1));}
        if let Screen::Diff(app)=&mut screens.views[0]{app.horizontal=[37;2];app.new_view=Some((req,dir.path().into()));}
        assert!(screens.poll());assert_eq!(screens.views.len(),2);assert_eq!(screens.active,1);
        let key=|code,modifiers|event::Event::Key(event::KeyEvent::new(code,modifiers));screens.handle(key(event::KeyCode::F(8),event::KeyModifiers::NONE));assert_eq!(screens.active,0);
        let Screen::Diff(app)=&screens.views[0]else{panic!()};assert_eq!(app.horizontal,[37;2]);
        screens.handle(key(event::KeyCode::F(8),event::KeyModifiers::SHIFT));assert_eq!(screens.active,1);screens.handle(key(event::KeyCode::Char('q'),event::KeyModifiers::NONE));screens.poll();assert_eq!(screens.views.len(),1);assert_eq!(screens.exit(),None);
        screens.handle(key(event::KeyCode::Char('q'),event::KeyModifiers::NONE));screens.poll();assert_eq!(screens.exit(),Some(0));
    }

    #[test]
    fn markdown_independent_views_start_source_and_close_pending_work() {
        use lenz::markdown::PresentationMode;
        let dir=tempfile::tempdir().unwrap();for (name,text) in [("left.md","# Old\n"),("right.md","# New\n")]{std::fs::write(dir.path().join(name),text).unwrap();}
        let request=||cli::parse(&["--no-index".into(),"left.md".into(),"right.md".into()],dir.path()).unwrap();let mut screens=Screens::open(request(),dir.path().into());
        let settle=|screens:&mut Screens|{for _ in 0..10000{screens.poll();if !screens.busy(){return;}std::thread::sleep(std::time::Duration::from_millis(1));}panic!("views busy");};settle(&mut screens);
        let key=|c|event::Event::Key(event::KeyEvent::new(c,event::KeyModifiers::NONE));
        let rendered=|screens:&mut Screens|{screens.handle(key(event::KeyCode::Char(':')));for c in "Inline".chars(){screens.handle(key(event::KeyCode::Char(c)));}screens.handle(key(event::KeyCode::Enter));};
        rendered(&mut screens);settle(&mut screens);
        let first=match &mut screens.views[0]{Screen::Diff(app)=>{app.new_view=Some((request(),dir.path().into()));app.session.markdown_projection().unwrap().clone()},_=>panic!()};screens.poll();settle(&mut screens);
        assert_eq!(screens.active,1);assert!(matches!(&screens.views[1],Screen::Diff(app)if app.session.markdown_mode()==PresentationMode::Source));
        rendered(&mut screens);settle(&mut screens);let second=match &screens.views[1]{Screen::Diff(app)=>app.session.markdown_projection().unwrap().clone(),_=>panic!()};assert!(!std::sync::Arc::ptr_eq(&first.pair,&second.pair));let weak=std::sync::Arc::downgrade(&second.pair);drop(second);
        screens.handle(key(event::KeyCode::Char('h')));assert!(screens.busy());screens.handle(key(event::KeyCode::Char('q')));screens.poll();assert_eq!(screens.views.len(),1);settle(&mut screens);assert!(weak.upgrade().is_none());
        assert!(matches!(&screens.views[0],Screen::Diff(app)if app.session.markdown_mode()==PresentationMode::Rendered));assert_eq!(std::fs::read(dir.path().join("right.md")).unwrap(),b"# New\n");
    }

    #[test]
    fn closing_a_view_or_exiting_deletes_its_terminal_images() {
        let dir=tempfile::tempdir().unwrap();
        for name in ["a.txt","b.txt"]{std::fs::write(dir.path().join(name),name).unwrap();}
        let request=||cli::parse(&["--no-index".into(),"a.txt".into(),"b.txt".into()],dir.path()).unwrap();
        let mut screens=Screens::open(request(),dir.path().into());
        screens.views.push(Screen::Diff(Box::new(app::App::open(request(),dir.path().into()))));
        let picture:md_images::Picture=std::sync::Arc::new(lenz::image_session::Pixels::new(8,8));
        let mut ids=Vec::new();
        for view in &mut screens.views {
            if let Screen::Diff(app)=view {app.markdown.images.enable_kitty();app.markdown.images.keep([(1usize,0usize,&picture,2u16,1u16)].into_iter());ids.push(app.markdown.images.show(1,0,&picture,2,1).unwrap());}
        }
        screens.take_graphics();
        screens.remove(1);
        assert!(screens.take_graphics().concat().contains(&format!("a=d,d=I,i={}",ids[1])),"a closed view's images");
        assert!(screens.shutdown_graphics().concat().contains(&format!("a=d,d=I,i={}",ids[0])),"and every view's at exit");
    }
}
