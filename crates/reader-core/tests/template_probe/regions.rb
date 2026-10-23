# Experiment only: ERB's installed scanner, not fixture-provided regions.
require "json"
input = JSON.parse(STDIN.read)
source = input.fetch("source")
if ["haml", "validate_haml"].include?(input.fetch("language"))
  require "haml"
  raise "unverified Haml version" unless Haml::VERSION == "7.5.1"
  ast = Haml::Parser.new({}).call(source)
  nodes = []
  walk = lambda do |node|
    unless [:root, :tag, :script, :silent_script, :plain].include?(node.type)
      raise "unsupported Haml context: #{node.type}"
    end
    if node.line
      nodes << { line: node.line, type: node.type, value: node.value.inspect }
    end
    node.children.each(&walk)
  end
  walk.call(ast)
  if input.fetch("language") == "validate_haml"
    compiled = Haml::Engine.new.call(source)
    raise "Haml compiler generated invalid Ruby" unless Prism.parse(compiled).errors.empty?
    puts JSON.generate({version: Haml::VERSION, nodes: nodes, compiled: compiled})
  else
    puts JSON.generate({version: Haml::VERSION, nodes: nodes})
  end
  exit
end
raise "unsupported template language" unless input.fetch("language") == "erb"
require "erb"
scanner = ERB::Compiler.new(nil).make_scanner(source)
regions = []
offset = 0
start = nil
scanner.scan do |token|
  next if token.nil? || token.empty?
  raise "scanner changed source spelling" unless source[offset, token.length] == token
  if scanner.stag.nil?
    if ["<%", "<%=", "<%#"].include?(token)
      scanner.stag = token
      start = offset + token.length
    end
  elsif token == "%>"
    regions << [start, offset] unless scanner.stag == "<%#"
    scanner.stag = nil
  elsif token == "%%>"
    raise "escaped ERB closing delimiter unsupported"
  end
  offset += token.length
end
regions << [start, source.length] if scanner.stag && scanner.stag != "<%#"
puts JSON.generate(regions)
