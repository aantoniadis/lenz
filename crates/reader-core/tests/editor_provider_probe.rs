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

fn sql_literal(text: &str) -> String { format!("'{}'",text.replace('\'',"''")) }
#[test]
fn edited_sql_is_one_literal_even_with_quotes_and_statements() {
    assert_eq!(sql_literal("'); DROP TABLE fixture; --"),"'''); DROP TABLE fixture; --'");
    assert_eq!(sql_literal("SELECT '😀'\r\n"),"'SELECT ''😀''\r\n'");
}

fn sql_fixtures(sentinel: &Path) -> Vec<(&'static str,String,Option<&'static str>)> {
    vec![
        ("alias","SELECT * FROM (SELECT 1 AS alias_col) a WHERE a.".into(),Some("alias_col")),
        ("cte","WITH c AS (SELECT 1 AS cte_col) SELECT * FROM c WHERE c.".into(),Some("cte_col")),
        ("nested","SELECT * FROM (SELECT * FROM (SELECT 1 AS nested_col) n) q WHERE q.".into(),Some("nested_col")),
        ("quoted","SELECT * FROM (SELECT 1 AS \"Mixed Column\") q WHERE q.".into(),Some("Mixed Column")),
        ("qualified","SELECT * FROM ready.".into(),Some("fixture")),
        ("qualified_prefix","SELECT * FROM ready.f".into(),Some("fixture")),
        ("function","SELECT ab".into(),Some("abs")),
        ("incomplete","WITH x AS (SELECT".into(),None),
        ("malformed","SELECT )".into(),None),
        ("duckdb","SELECT * EXCLUDE (id) FROM ready.fixture GROUP BY ALL".into(),None),
        ("postgres","SELECT DISTINCT ON (id) id::integer FROM ready.fixture ORDER BY id".into(),None),
        ("drop","DROP TABLE ready.fixture".into(),None),
        ("update","UPDATE ready.fixture SET label='changed'".into(),None),
        ("export",format!("COPY ready.fixture TO {}",sql_literal(&sentinel.to_string_lossy())),None),
        ("quote_escape","'); DROP TABLE ready.fixture; --".into(),None),
    ]
}
const DUCK_SETTINGS: &str = "SET autoinstall_known_extensions=false; SET autoload_known_extensions=false; SET enable_external_access=false; ";
fn duck(dir: &Path, database: &Path, readonly: bool, name: &str, statement: &str) -> (bool, Value) {
    let sql=format!("{DUCK_SETTINGS}{statement}");
    let mut command=Command::new(executable("python3"));
    command.args(["-c","import subprocess,resource,json,sys,pathlib; r=subprocess.run(sys.argv[2:]); pathlib.Path(sys.argv[1]).write_text(json.dumps({'max_rss_kib':resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss})); sys.exit(r.returncode)"]).arg(dir.join(format!("{name}-resource.json"))).arg(executable("duckdb"));
    command.args(["-no-agent","-init","/dev/null","-json","-echo","-bail"]).env("HOME",dir.join("home"));
    if readonly {command.arg("-readonly");}
    let started=Instant::now(); let result=command.arg(database).arg(&sql).output().unwrap();
    let stdout=String::from_utf8(result.stdout).unwrap();
    save(dir,&format!("{name}-boundary.json"),&json!({"database":database,"readonly":readonly,"statement":sql,"stdout":stdout,"stderr":String::from_utf8_lossy(&result.stderr),"success":result.status.success(),"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}));
    if result.status.success() && (statement.starts_with("SELECT * FROM sql_auto_complete(") || statement.starts_with("SELECT json_serialize_sql(")) {
        let echoed=stdout.split_once("SET enable_external_access=false; \n").expect("actual CLI statement echo").1;
        assert!(echoed.starts_with(&format!("{statement}\n")),"unexpected top-level statement boundary");
    }
    let rows=stdout.lines().position(|l|l.starts_with("[{"));
    let parsed=rows.and_then(|start|serde_json::from_str(&stdout.lines().skip(start).collect::<Vec<_>>().join("\n")).ok()).unwrap_or_else(||json!([]));
    (result.status.success(),parsed)
}
fn suggestions(rows: &Value, expected: &str) -> bool {
    rows.as_array().is_some_and(|rows|rows.iter().any(|r|r["suggestion"].as_str().is_some_and(|s|s.trim().trim_matches('"')==expected)))
}
#[test] #[ignore = "installed DuckDB; disposable metadata file and retained SQL boundary logs"]
fn duckdb_sql_assistance() {
    let dir=evidence("duckdb"); fs::create_dir_all(dir.join("home")).unwrap();
    let sentinel=dir.join("must-not-exist.csv"); assert!(!sentinel.exists());
    let fixtures=sql_fixtures(&sentinel); save(&dir,"fixtures.json",&json!(fixtures));
    let mut outcomes=Vec::new();
    let mut external_connections=0;
    let mut submitted=Vec::new();
    for (name,query,expected) in &fixtures {
        let statement=format!("SELECT * FROM sql_auto_complete({});",sql_literal(query)); submitted.push(statement.clone());
        let (ok,rows)=duck(&dir,Path::new(":memory:"),false,&format!("offline-{name}"),&statement); assert!(ok);
        outcomes.push(json!({"phase":"offline","fixture":name,"expected":expected,"expected_found":expected.map(|e|suggestions(&rows,e))}));
        if *name=="function" {assert!(suggestions(&rows,"abs"));assert_eq!(rows[0]["suggestion_start"],7);}
        let parse=format!("SELECT json_serialize_sql({}) AS parsed;",sql_literal(query)); submitted.push(parse.clone());
        let (ok,rows)=duck(&dir,Path::new(":memory:"),false,&format!("parse-{name}"),&parse); assert!(ok);
        if *name=="malformed" {assert_eq!(rows[0]["parsed"]["error"],true); assert_eq!(rows[0]["parsed"]["position"],"7");}
        if *name=="duckdb" {assert_eq!(rows[0]["parsed"]["error"],false);}
    }
    assert_eq!(external_connections,0,"offline requests must not open a metadata source");
    let database=dir.join("metadata.duckdb"); assert!(!database.exists(),"use a fresh evidence directory for each run");
    let setup="CREATE SCHEMA ready; CREATE TABLE ready.fixture(id INTEGER, label VARCHAR); INSERT INTO ready.fixture VALUES (7,'original'); CREATE MACRO ready.double_it(x) AS x*2;";
    fs::write(dir.join("setup.sql"),setup).unwrap(); assert!(duck(&dir,&database,false,"setup",setup).0);
    let before=duck(&dir,&database,true,"verify-before","SELECT * FROM ready.fixture;").1;
    let metadata="SELECT table_schema,table_name,column_name,data_type FROM information_schema.columns WHERE table_schema='ready' ORDER BY ordinal_position;";
    external_connections+=1;
    let (ok,columns)=duck(&dir,&database,true,"metadata",metadata); assert!(ok);assert_eq!(columns[0]["column_name"],"id");assert_eq!(columns[0]["data_type"],"INTEGER");
    for (name,query,expected) in &fixtures {
        external_connections+=1;
        let statement=format!("SELECT * FROM sql_auto_complete({});",sql_literal(query));submitted.push(statement.clone());
        let (ok,rows)=duck(&dir,&database,true,&format!("connected-{name}"),&statement);assert!(ok);
        outcomes.push(json!({"phase":"connected","fixture":name,"expected":expected,"expected_found":expected.map(|e|suggestions(&rows,e))}));
        if *name=="qualified_prefix" {assert!(suggestions(&rows,"fixture"));}
    }
    let functions=duck(&dir,&database,true,"functions","SELECT function_name,parameters,parameter_types,return_type,description FROM duckdb_functions() WHERE function_name IN ('abs','double_it');").1;
    assert!(functions.as_array().unwrap().iter().any(|r|r["function_name"]=="abs" && !r["parameter_types"].as_array().unwrap().is_empty()));
    let after=duck(&dir,&database,true,"verify-after","SELECT * FROM ready.fixture;").1; assert_eq!(before,after); assert!(!sentinel.exists());
    assert!(duck(&dir,&database,false,"refresh-setup","ALTER TABLE ready.fixture ADD COLUMN fresh_col INTEGER;").0);
    let refreshed=duck(&dir,&database,true,"refresh-metadata",metadata).1; assert!(refreshed.as_array().unwrap().iter().any(|r|r["column_name"]=="fresh_col"));
    let failed=duck(&dir,&dir.join("missing.duckdb"),true,"connection-failure",metadata);assert!(!failed.0);assert!(!dir.join("missing.duckdb").exists());
    let user_statements_submitted=submitted.iter().filter(|s|!(s.starts_with("SELECT * FROM sql_auto_complete('")||s.starts_with("SELECT json_serialize_sql('"))).count();
    let explain_statements_submitted=submitted.iter().filter(|s|s.trim_start().to_uppercase().starts_with("EXPLAIN")).count();
    assert_eq!(user_statements_submitted,0);assert_eq!(explain_statements_submitted,0);
    let format_input="select * exclude (id) from ready.fixture group by all;";
    let mut formatter=Command::new(executable("duckdb")).args(["-no-agent","-init","/dev/null","-format"]).env("HOME",dir.join("home")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    formatter.stdin.take().unwrap().write_all(format_input.as_bytes()).unwrap();
    let formatted=formatter.wait_with_output().unwrap();assert!(formatted.status.success());
    let formatted=String::from_utf8(formatted.stdout).unwrap();assert_eq!(formatted,"SELECT * EXCLUDE (id)\nFROM ready.fixture\nGROUP BY ALL;\n");
    save(&dir,"format.json",&json!({"input":format_input,"output":formatted,"database_open":false}));
    let version=Command::new(executable("duckdb")).arg("--version").output().unwrap();
    save(&dir,"summary.json",&json!({"version":String::from_utf8_lossy(&version.stdout),"outcomes":outcomes,"offline_external_connections":0,"connected_completion_and_catalog_opens":external_connections,"user_statements_submitted":user_statements_submitted,"explain_statements_submitted":explain_statements_submitted,"data_unchanged":before==after,"sentinel_absent":!sentinel.exists(),"formatting":"CLI -format produced expected DuckDB syntax; no edits applied","dialect_switch_cancellation":"unverified: no asynchronous SQL adapter","memory":"per-process maximum RSS in *-resource.json; no production worker measurements","refresh":"new readonly open sees explicit schema change"}));
    println!("DuckDB: {} fixture outcomes; query-local results are in summary.json",outcomes.len());
}

struct Pg { id: String, docker: PathBuf, dir: PathBuf }
impl Pg {
    fn start(docker: &Path, dir: &Path, name: &str) -> Result<Self, String> {
        let out=Command::new(docker).args(["run","--detach","--pull=never","--network","none","--name",name,"--tmpfs","/var/lib/postgresql:rw,size=512m","-e","POSTGRES_HOST_AUTH_METHOD=trust","postgres:18-alpine","-c","log_statement=all"]).output().map_err(|e|e.to_string())?;
        if !out.status.success() {return Err(String::from_utf8_lossy(&out.stderr).into_owned());}
        let id=String::from_utf8(out.stdout).map_err(|e|e.to_string())?.trim().to_owned();
        if id.len()!=64 || !id.bytes().all(|b|b.is_ascii_hexdigit()) {return Err("docker run did not return a container ID; cleanup is unarmed".into());}
        Ok(Self {id,docker:docker.into(),dir:dir.into()})
    }
    fn sql(&self, dir: &Path, phase: &str, sql: &str, readonly: bool) -> String {
        let mut c=Command::new(&self.docker); c.args(["exec","-i"]);
        if readonly {c.args(["-e","PGOPTIONS=-c default_transaction_read_only=on"]);}
        let t=Instant::now(); let out=c.args([&self.id,"psql","-X","-U","postgres","-v","ON_ERROR_STOP=1","-At","-c",sql]).output().unwrap();
        let text=String::from_utf8(out.stdout).unwrap();save(dir,&format!("{phase}-boundary.json"),&json!({"statement":sql,"readonly":readonly,"stdout":text,"stderr":String::from_utf8_lossy(&out.stderr),"success":out.status.success(),"elapsed_ms":t.elapsed().as_secs_f64()*1000.0}));assert!(out.status.success());text
    }
}
impl Drop for Pg {fn drop(&mut self){
    if let Ok(out)=Command::new(&self.docker).args(["logs",&self.id]).output(){let _=fs::write(self.dir.join("container-final.log"),[out.stdout,out.stderr].concat());}
    let _=Command::new(&self.docker).args(["rm","-f",&self.id]).output();
}}
#[test] #[ignore = "cached PostgreSQL container for metadata only; no PostgreSQL language provider installed"]
fn postgres_sql_assistance() {
    let dir=evidence("postgres");let sentinel=dir.join("must-not-exist.csv");assert!(!sentinel.exists());let fixtures=sql_fixtures(&sentinel);save(&dir,"fixtures.json",&json!(fixtures));
    let connections=0;assert_eq!(connections,0); // Offline provider absent: no language capabilities claimed.
    let image=Command::new("docker").args(["image","inspect","postgres:18-alpine","--format","{{.Id}}"]).output().unwrap();
    assert!(image.status.success(),"unverified: no cached image");
    let pg=Pg::start(&executable("docker"),&dir,&format!("lenz-readiness-{}",std::process::id())).unwrap();
    let deadline=Instant::now()+Duration::from_secs(30);
    loop {
        let init=Command::new("docker").args(["exec",&pg.id,"cat","/proc/1/comm"]).output().unwrap();
        if init.stdout==b"postgres\n" {
            let ready=Command::new("docker").args(["exec",&pg.id,"pg_isready","-U","postgres"]).output().unwrap();
            if ready.status.success(){break;}
        }
        assert!(Instant::now()<deadline,"PostgreSQL readiness timeout");std::thread::sleep(Duration::from_millis(100));
    }
    let version=pg.sql(&dir,"version","SHOW server_version;",true);
    let setup="CREATE SCHEMA ready; CREATE TABLE ready.fixture(id integer, label text); INSERT INTO ready.fixture VALUES (7,'original'); CREATE FUNCTION ready.double_it(x integer) RETURNS integer LANGUAGE SQL IMMUTABLE AS 'SELECT x*2';";
    fs::write(dir.join("setup.sql"),setup).unwrap();pg.sql(&dir,"setup",setup,false);
    let before=pg.sql(&dir,"verify-before","SELECT * FROM ready.fixture;",true);
    let metadata="SELECT column_name,data_type FROM information_schema.columns WHERE table_schema='ready' ORDER BY ordinal_position;";
    let columns=pg.sql(&dir,"metadata",metadata,true);assert_eq!(columns,"id|integer\nlabel|text\n");
    let functions=pg.sql(&dir,"functions","SELECT proname,pg_get_function_identity_arguments(p.oid),pg_get_function_result(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='ready';",true);assert!(functions.contains("double_it|x integer|integer"));
    // Edited SQL is never passed to this catalog-only adapter; fixtures trigger the same fixed query.
    for (name,_,_) in &fixtures {assert_eq!(pg.sql(&dir,&format!("fixture-{name}"),metadata,true),columns);}
    let after=pg.sql(&dir,"verify-after","SELECT * FROM ready.fixture;",true);assert_eq!(before,after);assert!(!sentinel.exists());
    pg.sql(&dir,"refresh-setup","ALTER TABLE ready.fixture ADD COLUMN fresh_col integer;",false);assert!(pg.sql(&dir,"refresh",metadata,true).contains("fresh_col|integer"));
    let stats=Command::new("docker").args(["stats","--no-stream","--format","{{json .}}",&pg.id]).output().unwrap();fs::write(dir.join("container-memory.json"),stats.stdout).unwrap();
    let failed=Command::new("docker").args(["exec","-e","PGOPTIONS=-c default_transaction_read_only=on",&pg.id,"psql","-X","-U","postgres","-d","missing_readiness_database","-At","-c",metadata]).output().unwrap();assert!(!failed.status.success());fs::write(dir.join("connection-failure.log"),failed.stderr).unwrap();
    let logs=Command::new("docker").args(["logs",&pg.id]).output().unwrap();let log=String::from_utf8([logs.stdout,logs.stderr].concat()).unwrap();fs::write(dir.join("server-statements.log"),&log).unwrap();
    let user_statements_submitted=fixtures.iter().filter(|(_,sql,_)|log.lines().any(|l|l.split_once("statement: ").is_some_and(|(_,actual)|actual.trim_end_matches(';')==sql))).count();
    let explain_statements_submitted=log.lines().filter(|l|l.contains("statement: EXPLAIN")).count();assert_eq!(user_statements_submitted,0);assert_eq!(explain_statements_submitted,0);
    save(&dir,"summary.json",&json!({"version":version.trim(),"image":String::from_utf8_lossy(&image.stdout).trim(),"offline_connections":connections,"language_provider":"unverified: none installed; all offline language requirements remain blocked","metadata":"columns/types and function signatures demonstrated","user_statements_submitted":user_statements_submitted,"explain_statements_submitted":explain_statements_submitted,"data_unchanged":before==after,"sentinel_absent":!sentinel.exists(),"refresh":"explicit query sees schema change","connection_failure":"missing database refused","memory":"one container cgroup sample in container-memory.json, not process RSS","dialect_switch_cancellation":"unverified"}));
    println!("PostgreSQL {version}: metadata boundary checked; language provider absent");
}

#[test]
fn postgres_cleanup_requires_successful_acquisition() {
    use std::os::unix::fs::PermissionsExt;
    let temp=tempfile::tempdir().unwrap();
    let docker=temp.path().join("docker");
    fs::write(&docker,r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.calls"
if [ "$1" = run ]; then exit 1; fi
"#).unwrap();
    fs::set_permissions(&docker,fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Pg::start(&docker,temp.path(),"existing-container").is_err());
    let calls=fs::read_to_string(temp.path().join("docker.calls")).unwrap();
    assert_eq!(calls.lines().count(),1,"failed creation must not inspect or remove an unowned container: {calls}");
}

#[test]
fn postgres_cleanup_uses_the_acquired_id() {
    use std::os::unix::fs::PermissionsExt;
    let temp=tempfile::tempdir().unwrap();let docker=temp.path().join("docker");let id="a".repeat(64);
    fs::write(&docker,format!(r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.calls"
if [ "$1" = run ]; then printf '%s\n' '{id}'; fi
"#)).unwrap();
    fs::set_permissions(&docker,fs::Permissions::from_mode(0o755)).unwrap();
    let pg=Pg::start(&docker,temp.path(),"reusable-name").unwrap();assert_eq!(pg.id,id);drop(pg);
    let calls=fs::read_to_string(temp.path().join("docker.calls")).unwrap();
    assert_eq!(calls.lines().skip(1).collect::<Vec<_>>(),vec![format!("logs {id}"),format!("rm -f {id}")]);
}
