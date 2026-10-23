//! Readiness experiments only; generated regions are fixture inputs, not a template parser.
#![cfg(unix)]
use std::{ops::Range, path::PathBuf};
use reader_core::{document::Document, semantics::lsp::{self, Encoding}};
use serde_json::{Value, json};

struct Projection { source: String, generated: String, spans: Vec<(Range<usize>, Range<usize>)>, version: u64 }
impl Projection {
    fn new(source: &str, regions: &[&str]) -> Self {
        let mut generated = String::from("# generated wrapper\n");
        let mut spans = Vec::new();
        for region in regions {
            let start = source.find(region).unwrap();
            let at = generated.len();
            generated.push_str(region);
            spans.push((start..start + region.len(), at..generated.len()));
            generated.push('\n');
        }
        Self { source: source.into(), generated, spans, version: 2 }
    }
    fn map(&self, range: &Value, version: u64, uri: &str) -> Option<Range<usize>> { self.map_encoded(range,version,uri,Encoding::Utf16) }
    fn map_encoded(&self, range: &Value, version: u64, uri: &str, encoding: Encoding) -> Option<Range<usize>> {
        if version != self.version || uri != "fixture" { return None; }
        let doc = Document::decode(PathBuf::from("projection.rb"), version, self.generated.as_bytes());
        let start = lsp::offset(&doc, &range["start"], encoding)?;
        let end = lsp::offset(&doc, &range["end"], encoding)?;
        if start > end || lsp::position(&doc, start, encoding) != range["start"] || lsp::position(&doc, end, encoding) != range["end"] { return None; }
        self.spans.iter().find_map(|(source, generated)| {
            (start >= generated.start && end <= generated.end).then(|| source.start + start-generated.start..source.start+end-generated.start)
        })
    }
}
fn range(text: &str, bytes: Range<usize>) -> Value { encoded_range(text,bytes,Encoding::Utf16) }
fn encoded_range(text: &str, bytes: Range<usize>, encoding: Encoding) -> Value {
    let doc = Document::decode(PathBuf::from("projection.rb"), 2, text.as_bytes());
    json!({"start":lsp::position(&doc, bytes.start, encoding), "end":lsp::position(&doc, bytes.end, encoding)})
}
#[test]
fn template_mapping_preserves_original_bytes() {
    for source in ["<div title=\"😀\"><%= \"😀\"; Widget.gre %></div>\r\n<% x =\r\n  1 %>", "%div{title: \"😀\"}\r\n  = \"😀\"; Widget.gre\r\n  %p #{x =\r\n  1}"] {
        let p = Projection::new(source, &["\"😀\"; Widget.gre", "x =\r\n  1"]);
        let at = p.generated.find("gre").unwrap();
        let expected = source.find("gre").unwrap();
        let mapped = p.map(&range(&p.generated, at..at+3), 2, "fixture").unwrap();
        assert_eq!(mapped, expected..expected+3);
        let mut edited = p.source.clone();
        edited.replace_range(mapped.clone(), "greet");
        assert_eq!(&edited[..mapped.start], &source[..mapped.start]);
        assert_eq!(&edited[mapped.start+5..], &source[mapped.end..]);
        let multiline = p.generated.find("x =").unwrap();
        assert_eq!(p.map(&range(&p.generated, multiline..multiline+8), 2, "fixture"), Some(source.find("x =").unwrap()..source.find("x =").unwrap()+8));
    }
}
#[test]
fn projection_rejects_scaffolding_stale_and_foreign_edits() {
    let p = Projection::new("%p= Widget.gre", &["Widget.gre"]);
    let at = p.generated.find("gre").unwrap();
    assert!(p.map(&range(&p.generated, 0..at+3), 2, "fixture").is_none());
    assert!(p.map(&range(&p.generated, at..at+3), 1, "fixture").is_none());
    assert!(p.map(&range(&p.generated, at..at+3), 2, "other").is_none());
    assert!(p.map(&json!({"start":{"line":999,"character":0},"end":{"line":999,"character":1}}), 2,"fixture").is_none());
}

use std::{fs, io::{BufReader, Write}, path::Path, process::{Child, ChildStdin, Command, Stdio}, sync::mpsc, time::{Duration, Instant}};
fn evidence(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join(std::env::var_os("EDITOR_EVIDENCE_DIR").unwrap_or_else(|| "target/perf/editor-readiness/providers".into())).join(name);
    fs::create_dir_all(&dir).unwrap(); dir.canonicalize().unwrap()
}
fn save(dir: &Path, name: &str, value: &Value) { fs::write(dir.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap(); }
fn executable(name: &str) -> PathBuf { reader_core::semantics::provider::find_executable(Path::new(name), None).unwrap_or_else(|| panic!("unverified: missing {name}")) }
struct Rpc { child: Child, input: ChildStdin, messages: mpsc::Receiver<Value>, log: fs::File, id: u64, started: Instant, encoding: Encoding }
impl Rpc {
    fn start(mut command: Command, dir: &Path, name: &str) -> Self {
        fs::create_dir_all(dir.join("home")).unwrap();
        command.env("HOME", dir.join("home")).env("XDG_CACHE_HOME", dir.join("cache")).env("XDG_CONFIG_HOME", dir.join("config"));
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(fs::File::create(dir.join(format!("{name}-stderr.log"))).unwrap()).spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = mpsc::sync_channel(128);
        std::thread::spawn(move || { let mut r = BufReader::new(stdout); while let Some(v) = lsp::read_message(&mut r) { if tx.send(v).is_err() { break; } } });
        Self { child, input, messages, log: fs::File::create(dir.join(format!("{name}-rpc.jsonl"))).unwrap(), id: 0, started: Instant::now(), encoding: Encoding::Utf16 }
    }
    fn send(&mut self, value: Value) { writeln!(self.log, "{}", json!({"direction":"send","ms":self.started.elapsed().as_secs_f64()*1000.0,"message":value})).unwrap(); lsp::write_message(&mut self.input,&value).unwrap(); }
    fn notify(&mut self, method: &str, params: Value) { self.send(json!({"jsonrpc":"2.0","method":method,"params":params})); }
    fn ask(&mut self, method: &str, params: Value) -> Value {
        self.id += 1; let id = self.id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        let deadline = Instant::now()+Duration::from_secs(30);
        loop {
            let v = self.messages.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|e| panic!("{method}: {e}"));
            writeln!(self.log,"{}",json!({"direction":"receive","ms":self.started.elapsed().as_secs_f64()*1000.0,"message":v})).unwrap();
            if let Some(method) = v["method"].as_str() {
                if !v["id"].is_null() {
                    let result = match method {
                        "workspace/configuration" => json!(v["params"]["items"].as_array().unwrap().iter().map(|_|json!({})).collect::<Vec<_>>()),
                        "workspace/applyEdit" => json!({"applied":false,"failureReason":"readiness probe"}),
                        _ => Value::Null,
                    };
                    self.send(json!({"jsonrpc":"2.0","id":v["id"],"result":result}));
                }
            } else if v["id"] == id { return v; }
        }
    }
    fn init(&mut self, root: &Path) -> Value {
        let r = self.ask("initialize",json!({"processId":std::process::id(),"rootUri":lsp::file_uri(root),"capabilities":{"general":{"positionEncodings":["utf-8","utf-16"]},"experimental":{"requestDelegation":true},"textDocument":{"completion":{"completionItem":{"snippetSupport":true}},"diagnostic":{}},"workspace":{"configuration":true}},"initializationOptions":{"formatter":"none","provideFormatter":true}}));
        self.encoding = Encoding::negotiated(&r["result"]["capabilities"]); self.notify("initialized",json!({})); r
    }
    fn open(&mut self, path: &Path, lang: &str, text: &str) { self.notify("textDocument/didOpen",json!({"textDocument":{"uri":lsp::file_uri(path),"languageId":lang,"version":1,"text":text}})); }
    fn change(&mut self, path: &Path, old: &str, text: &str, version: u64) { self.notify("textDocument/didChange",json!({"textDocument":{"uri":lsp::file_uri(path),"version":version},"contentChanges":[{"range":encoded_range(old,0..old.len(),self.encoding),"text":text}]})); }
    fn at(&mut self, method: &str, path: &Path, text: &str, needle: &str) -> Value {
        let at = text.find(needle).unwrap()+needle.len();
        let doc = Document::decode(path.into(),2,text.as_bytes());
        self.ask(method,json!({"textDocument":{"uri":lsp::file_uri(path)},"position":lsp::position(&doc,at,self.encoding)}))
    }
    fn rss(&self) -> Option<u64> { fs::read_to_string(format!("/proc/{}/status",self.child.id())).ok()?.lines().find_map(|l|l.strip_prefix("VmRSS:")?.split_whitespace().next()?.parse().ok()) }
}
impl Drop for Rpc { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }
fn items(v: &Value) -> &[Value] { v["result"].as_array().or_else(||v["result"]["items"].as_array()).map_or(&[], Vec::as_slice) }
fn ruby(dir: &Path) -> Rpc {
    fs::write(dir.join("Gemfile"),"source 'https://rubygems.org'\ngem 'ruby-lsp', '=0.26.11'\n").unwrap();
    let paths = Command::new(executable("ruby")).args(["-e","puts Gem.path.join(':')"]).output().unwrap();
    assert!(paths.status.success()); let paths = String::from_utf8(paths.stdout).unwrap();
    fs::create_dir_all(dir.join("home")).unwrap();
    let mut command = Command::new(executable("bundle"));
    command.current_dir(dir).env("HOME",dir.join("home")).env("GEM_PATH",paths.trim()).env("BUNDLE_GEMFILE",dir.join("Gemfile")).env("BUNDLE_USER_HOME",dir.join("bundle-home")).env("BUNDLE_IGNORE_CONFIG","1");
    let locked = command.args(["lock","--local"]).output().unwrap();
    fs::write(dir.join("bundle-local.log"),[locked.stdout,locked.stderr].concat()).unwrap(); assert!(locked.status.success(),"unverified: local bundle unavailable");
    let mut c = Command::new(executable("ruby-lsp"));
    c.current_dir(dir).env("GEM_PATH",paths.trim()).env("BUNDLE_GEMFILE",dir.join("Gemfile")).env("BUNDLE_USER_HOME",dir.join("bundle-home")).env("BUNDLE_IGNORE_CONFIG","1").env("RUBYOPT","-rbundler/setup");
    Rpc::start(c,dir,"ruby")
}
fn html(dir: &Path) -> Rpc {
    let server = "/usr/share/code/resources/app/extensions/html-language-features/server/dist/node/htmlServerMain.js";
    assert!(Path::new(server).is_file(),"unverified: bundled HTML server absent");
    let mut c = Command::new(executable("node")); c.args([server,"--stdio"]).current_dir(dir); Rpc::start(c,dir,"html")
}
fn template_assistance(haml: bool) {
    let dir = evidence(if haml {"haml"} else {"erb"});
    let helper = "class ReadyWidget\n  def self.greet(name, excited: false)\n    name\n  end\nend\n";
    fs::write(dir.join("helper.rb"),helper).unwrap();
    let original = if haml { "%div{title: \"😀\"}\r\n  = \"😀\"; ReadyWidget.gre\r\n" } else { "<div title=\"😀\"><%= \"😀\"; ReadyWidget.gre %></div>\r\n" };
    let path = dir.join(if haml {"view.html.haml"} else {"view.html.erb"}); fs::write(&path,original).unwrap();
    let mut projection = Projection::new(original,&["\"😀\"; ReadyWidget.gre"]);
    projection.version = 1;
    let ruby_path = if haml {dir.join("projection.rb")} else {path.clone()};
    let source = if haml {projection.generated.as_str()} else {original};
    let mut ruby = ruby(&dir); let init = ruby.init(&dir); save(&dir,"ruby-initialize.json",&init);
    ruby.open(&dir.join("helper.rb"),"ruby",helper); ruby.open(&ruby_path,if haml {"ruby"} else {"erb"},source);
    let started = Instant::now();
    let completion = loop {
        let r = ruby.at("textDocument/completion",&ruby_path,source,"ReadyWidget.gre");
        if !items(&r).is_empty() || started.elapsed()>Duration::from_secs(15) {break r;}
        std::thread::sleep(Duration::from_millis(100));
    };
    save(&dir,"member-completion.json",&completion);
    let signature_text = source.replace("ReadyWidget.gre","ReadyWidget.greet(");
    ruby.change(&ruby_path,source,&signature_text,2);
    let signature = ruby.at("textDocument/signatureHelp",&ruby_path,&signature_text,"ReadyWidget.greet("); save(&dir,"signature.json",&signature);
    let invalid = source.replace("ReadyWidget.gre","ReadyWidget.greet(]"); ruby.change(&ruby_path,&signature_text,&invalid,3);
    let diagnostic = ruby.ask("textDocument/diagnostic",json!({"textDocument":{"uri":lsp::file_uri(&ruby_path)}})); save(&dir,"diagnostic.json",&diagnostic);
    let mut mapping = Vec::new();
    for item in items(&completion) {
        if let Some(edit) = item.get("textEdit") {
            let r = edit.get("range").or_else(||edit.get("replace")).unwrap_or(&Value::Null);
            if haml { mapping.push(json!({"label":item["label"],"source_bytes":projection.map_encoded(r,1,"fixture",ruby.encoding).map(|r|[r.start,r.end])})); }
            else { let doc=Document::decode(path.clone(),1,source.as_bytes()); mapping.push(json!({"label":item["label"],"source_bytes":[lsp::offset(&doc,&r["start"],ruby.encoding),lsp::offset(&doc,&r["end"],ruby.encoding)]})); }
        }
    }
    let member = items(&completion).iter().find(|v|v["label"]=="greet").expect("expected helper member");
    let replace = &member["textEdit"]["range"];
    let expected = original.find("gre").unwrap();
    let mapped = if haml { projection.map_encoded(replace,1,"fixture",ruby.encoding) } else {
        let doc=Document::decode(path.clone(),1,source.as_bytes());
        Some(lsp::offset(&doc,&replace["start"],ruby.encoding).unwrap()..lsp::offset(&doc,&replace["end"],ruby.encoding).unwrap())
    };
    if haml { assert_eq!(mapped,Some(expected..expected+3)); }
    save(&dir,"native-range-check.json",&json!({"expected":[expected,expected+3],"actual":mapped.as_ref().map(|r|[r.start,r.end]),"safe":mapped==Some(expected..expected+3)}));
    if haml { assert!(projection.map_encoded(replace,2,"fixture",ruby.encoding).is_none()); }
    // Native ERB diagnostics are absent in this server; evaluate the same explicit Ruby projection route.
    let invalid_original=original.replace("ReadyWidget.gre","ReadyWidget.greet(]");
    let mut diagnostic_projection=Projection::new(&invalid_original,&["\"😀\"; ReadyWidget.greet(]"]);
    diagnostic_projection.version=3;
    let diagnostic_path=dir.join("diagnostic-projection.rb");
    ruby.open(&diagnostic_path,"ruby",&projection.generated);
    let projected_completion=ruby.at("textDocument/completion",&diagnostic_path,&projection.generated,"ReadyWidget.gre");
    save(&dir,"projected-completion.json",&projected_completion);
    let projected_member=items(&projected_completion).iter().find(|v|v["label"]=="greet").expect("projected helper member");
    assert_eq!(projection.map_encoded(&projected_member["textEdit"]["range"],1,"fixture",ruby.encoding),Some(expected..expected+3));
    ruby.change(&diagnostic_path,&projection.generated,&diagnostic_projection.generated,3);
    let projected_diagnostics=ruby.ask("textDocument/diagnostic",json!({"textDocument":{"uri":lsp::file_uri(&diagnostic_path)}}));
    save(&dir,"projected-diagnostics.json",&projected_diagnostics);
    let diagnostic_ranges:Vec<_>=projected_diagnostics["result"]["items"].as_array().unwrap().iter().filter_map(|d|{
        let mapped=diagnostic_projection.map_encoded(&d["range"],3,"fixture",ruby.encoding)?;
        Some(json!({"message":d["message"],"severity":d["severity"],"source_bytes":[mapped.start,mapped.end],"source_text":&invalid_original[mapped]}))
    }).collect();
    assert!(diagnostic_ranges.iter().any(|d|d["severity"]==1 && d["source_text"].as_str().unwrap().contains(']')),"missing mapped syntax diagnostic");
    save(&dir,"mapped-diagnostics.json",&json!(diagnostic_ranges));
    let mut html = html(&dir); let html_init=html.init(&dir); save(&dir,"html-initialize.json",&html_init);
    let html_path=dir.join("projection.html"); let tag="<di"; html.open(&html_path,"html",tag);
    let tags=html.at("textDocument/completion",&html_path,tag,"di"); save(&dir,"html-tags.json",&tags);
    let attr="<div cla"; html.change(&html_path,tag,attr,2); let attrs=html.at("textDocument/completion",&html_path,attr,"cla"); save(&dir,"html-attributes.json",&attrs);
    assert!(items(&tags).iter().any(|v|v["label"]=="div"));
    assert!(items(&attrs).iter().any(|v|v["label"]=="class"));
    let delegation = if haml {Value::Null} else {ruby.change(&ruby_path,&invalid,"<di",4);ruby.at("textDocument/completion",&ruby_path,"<di","di")}; save(&dir,"delegation.json",&delegation);
    // Keep honest outcomes: a responding provider with empty assistance is not a capability pass.
    let summary=json!({"kind":if haml {"haml"} else {"erb"},"ruby_server":init["result"]["serverInfo"],"member_count":items(&completion).len(),"signature_count":signature["result"]["signatures"].as_array().map_or(0,Vec::len),"diagnostic_count":diagnostic["result"]["items"].as_array().map_or(0,Vec::len),"mapped_completion":mapping,"html_tag_count":items(&tags).len(),"html_attribute_count":items(&attrs).len(),"ruby_rss_kib":ruby.rss(),"html_rss_kib":html.rss(),"elapsed_seconds":started.elapsed().as_secs_f64(),"rails":"unverified: Rails and ruby-lsp-rails are not installed","haml_grammar":"unverified: explicit fixture regions only","obsolete_reply":"rejected by deterministic mapping test; no production request lifecycle tested"});
    save(&dir,"summary.json",&summary); println!("{summary}");
    assert_eq!(fs::read_to_string(&path).unwrap(),original,"draft requests must not write source");
}
#[test] #[ignore = "real installed Ruby/HTML providers; retained private fixtures"]
fn erb_provider_assistance() { template_assistance(false); }
#[test] #[ignore = "test-only Haml projection, not a Haml provider"]
fn haml_provider_assistance() { template_assistance(true); }

fn projected_edits(p: &Projection, edits: &[Value], version: u64) -> Option<String> {
    let mut out = p.source.clone();
    let mut mapped = edits.iter().map(|e| Some((p.map(&e["range"],version,e["uri"].as_str()?)?,e["newText"].as_str()?))).collect::<Option<Vec<_>>>()?;
    mapped.sort_by_key(|(r,_)|r.start);
    if mapped.windows(2).any(|w|w[0].0.end > w[1].0.start) {return None;}
    for (range,text) in mapped.into_iter().rev() {out.replace_range(range,text);}
    Some(out)
}
#[test]
fn projected_transaction_rejects_every_edit_if_one_is_unsafe() {
    let p=Projection::new("%p= Widget.gre", &["Widget.gre"]);
    let at=p.generated.find("gre").unwrap();
    let valid=json!({"range":range(&p.generated,at..at+3),"uri":"fixture","newText":"greet"});
    assert_eq!(projected_edits(&p,std::slice::from_ref(&valid),2).unwrap(),"%p= Widget.greet");
    for invalid in [json!({"range":range(&p.generated,0..1),"uri":"fixture","newText":"x"}), json!({"range":valid["range"],"uri":"other","newText":"x"})] {
        assert!(projected_edits(&p,&[valid.clone(),invalid],2).is_none());
        assert_eq!(p.source,"%p= Widget.gre");
    }
    assert!(projected_edits(&p,&[valid],1).is_none());
}
