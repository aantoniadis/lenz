//! Template projections against the template engines themselves (ticket 57): every region Lenz maps as a template's
//! Ruby is Ruby the engine compiles, verbatim, and the engine's compiled Ruby parses. Opt-in: they need Ruby with Action
//! View (Rails' ERB handler, on Erubi) and, for Haml, Haml 7.5.1 provisioned in an owned gem directory named by
//! `EDITOR_HAML_GEM_HOME` (`gem install haml -v 7.5.1 --install-dir "$EDITOR_HAML_GEM_HOME" --no-document`).
#![cfg(unix)]

use std::{io::Write, path::PathBuf, process::{Command, Stdio}};

use lenz_content::highlight::Language;
use code_core::document::Document;

/// Run `script` with `source` on its input, under `gem_home` when given; its output.
fn ruby(script: &str, source: &str, gem_home: Option<&str>) -> String {
    let mut command = Command::new("ruby");
    command.args(["-E", "UTF-8:UTF-8", "-e", script]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(home) = gem_home { command.env("GEM_HOME", home).env_remove("GEM_PATH").env("RUBYOPT", ""); }
    let mut child = command.spawn().expect("Ruby is required");
    child.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// The Ruby regions Lenz maps in template `name`.
fn regions(name: &str, source: &str) -> Vec<String> {
    let doc = Document::decode(PathBuf::from(format!("/fixture/{name}")), 1, source.as_bytes());
    let views = doc.templates().unwrap();
    let ruby = views.language(Language::Ruby).unwrap_or_else(|| panic!("{name}: {:?}", views.primary().err())).embedded().unwrap();
    ruby.spans().iter().map(|s| source[s.source.clone()].to_owned()).collect()
}

/// `compiled` holds every region verbatim (a `|` continuation's line as Haml joins it: its text, trimmed).
fn assert_compiled(name: &str, regions: &[String], compiled: &str) {
    for region in regions {
        assert!(compiled.contains(region.as_str()) || compiled.contains(region.trim()), "{name}: region {region:?} is not in the engine's Ruby:\n{compiled}");
    }
}

const PARSES: &str = "require 'prism'; src = STDIN.read; errors = Prism.parse(src).errors; abort(errors.map(&:message).join(\"\\n\") + \"\\n\" + src) unless errors.empty?";

const ERB: [(&str, &str); 3] = [
    ("greet.html.erb", "<ul class=\"𝄞\">\r\n<% names = [\"é\", \"𝄞\"] %>\r\n<li title=\"<%= Greeter.greet(\"𝄞\") %>\">𝄞 <%= Greeter.greet %></li>\r\n<li><%= Greeter.greet(names.first, excited: true) %></li>\r\n<% total = [1,\r\n  2].sum %>\r\n<p>𝄞 </p>\r\n<%= total %>\r\n</ul>\r\n"),
    ("trim.html.erb", "<%- if true -%>\n  <%== \"<b>\" =%>\n<%# a comment with <%= nothing %>\n<%% literal %>\n<% end -%>\n<%= [1, 2].map do |n| %><%= n %><% end %>\n"),
    ("mail.text.erb", "Hi <%= \"é\" + \"𝄞\" %>,\r\n<% 3.times do |i| %>line <%= i %>\r\n<% end %>\r\n"),
];

/// Every ERB region is code Rails' own ERB handler (Action View's Erubi, as Rails runs it) compiles verbatim, and its
/// Ruby parses.
#[test]
#[ignore = "needs Ruby with Action View (Rails' ERB handler); run alone"]
fn real_erb_regions_are_the_ruby_rails_compiles() {
    for (name, source) in ERB {
        let found = regions(name, source);
        assert!(!found.is_empty(), "{name}");
        let compiled = ruby("require 'action_view'; print ActionView::Template::Handlers::ERB::Erubi.new(STDIN.read, trim: true).src", source, None);
        assert_compiled(name, &found, &compiled);
        ruby(PARSES, &compiled, None);
        println!("{name}: {} regions, all in Action View's Ruby", found.len());
    }
}

const HAML: [(&str, &str); 4] = [
    ("list.html.haml", "%ul.list(title=\"😀\")\r\n  - names = [\"é\", \"𝄞\"]\r\n  %li= Greeter.greet(\"𝄞\")\r\n  %li= Greeter.greet\r\n  %li Hi #{Greeter.greet(names.first, excited: true)} 𝄞\r\n  -# a comment\r\n    %li= ignored(\r\n"),
    ("multi.html.haml", "!!!\n%html\n  %body.page#main(class=\"x\")\n    %p= Greeter.greet(\"é\", |\n        excited: true) |\n    .note= 1 + 2\n    %p Hi #{Greeter.greet(\"𝄞\")}, #{names = 1}\n    %br/\n"),
    ("nested.html.haml", "%ul\r\n  - items.each do |item|\r\n    - if item\r\n      %li{title: item.title}= item.name\r\n    - else\r\n      %li Hello #{item.fetch({name: \"𝄞\"})}\r\n"),
    ("attributes.html.haml", "%a(href=url title=\"#{item.title}\") Link\n= greet(\"𝄞\",\n  excited: true)\n"),
];

/// Every Haml region is Ruby Haml 7.5.1 itself compiles (script and interpolation code verbatim), and its Ruby parses.
#[test]
#[ignore = "needs Ruby and Haml 7.5.1 in an owned gem directory named by EDITOR_HAML_GEM_HOME; run alone"]
fn real_haml_regions_are_the_ruby_haml_compiles() {
    let home = std::env::var("EDITOR_HAML_GEM_HOME").expect("set EDITOR_HAML_GEM_HOME to an owned gem directory holding Haml 7.5.1");
    for (name, source) in HAML {
        let found = regions(name, source);
        assert!(!found.is_empty(), "{name}");
        let compiled = ruby("require 'haml'; abort('Haml 7.5.1 required') unless Haml::VERSION == '7.5.1'; print Haml::Engine.new.call(STDIN.read)", source, Some(&home));
        // Haml optimizes attribute hashes rather than copying them into the compiled Ruby. Its
        // parser retains those exact Ruby hashes as dynamic_attributes; check against that source too.
        let attributes = ruby("require 'haml'; root = Haml::Parser.new({}).call(STDIN.read); walk = ->(n) { a = n.value.is_a?(Hash) && n.value[:dynamic_attributes]; puts a.old if a && a.old; n.children.each { |c| walk.call(c) } }; walk.call(root)", source, Some(&home));
        assert_compiled(name, &found, &format!("{compiled}\n{attributes}"));
        ruby(PARSES, &compiled, None);
        let doc = Document::decode(PathBuf::from(name), 1, source.as_bytes());
        let projection = doc.templates().unwrap().primary().unwrap().embedded().unwrap().generated();
        ruby(PARSES, projection.text(), None);
        println!("{name}: {} regions, all in Haml's Ruby", found.len());
    }
}
