# Private version-scoped experiment; never change installed gems or production configuration.
require "ruby_lsp/internal"
raise "unverified Ruby LSP version" unless RubyLsp::VERSION == "0.26.11"

class TemplateSignatureListener < RubyLsp::Listeners::SignatureHelp
  def initialize(active_parameter, *arguments)
    @template_active_parameter = active_parameter
    super(*arguments)
  end

  private

  def determine_active_signature_and_parameter(node, signatures)
    signature, = super
    maximum = [signatures[signature].parameters.length - 1, 0].max
    [signature, [@template_active_parameter, maximum].min]
  end
end

module TemplateSignatureRequest
  def initialize(document, global_state, position, _context, dispatcher, sorbet_level)
    raise "experiment requires a UTF-8 Ruby projection" unless document.is_a?(RubyLsp::RubyDocument) && document.encoding == Encoding::UTF_8
    @dispatcher = dispatcher
    @response_builder = RubyLsp::ResponseBuilders::SignatureHelp.new
    character, = document.find_index_by_position(position)
    cursor = document.source[0...character].bytesize
    candidates = []
    pending = [document.ast]
    until pending.empty?
      node = pending.pop
      pending.concat(node.child_nodes.compact)
      next unless node.is_a?(Prism::CallNode) && node.opening_loc&.slice == "("
      next if cursor < node.opening_loc.end_offset
      closing = node.closing_loc
      next if closing && closing.length > 0 && cursor > closing.start_offset
      if cursor > node.location.end_offset
        next unless document.source.byteslice(node.location.end_offset...cursor).strip.empty?
      end
      candidates << node
    end
    @target = candidates.max_by { |node| node.opening_loc.start_offset }
    return unless @target

    # Target the method name, not the argument boundary that Ruby LSP adjusts to its parent.
    at = document.source.byteslice(0...@target.message_loc.start_offset).length
    node_context = RubyLsp::RubyDocument.locate(document.ast, at,
      node_types: [Prism::CallNode], code_units_cache: document.code_units_cache)
    prefix = document.source.byteslice(@target.opening_loc.end_offset...cursor)
    depth = 0
    parameter = 0
    Prism.lex(prefix).value.each do |token, _|
      case token.type
      when :PARENTHESIS_LEFT, :BRACKET_LEFT, :BRACE_LEFT, :LAMBDA_BEGIN, :EMBEXPR_BEGIN, :KEYWORD_DO
        depth += 1
      when :PARENTHESIS_RIGHT, :BRACKET_RIGHT, :BRACE_RIGHT, :EMBEXPR_END, :KEYWORD_END
        depth -= 1
      when :COMMA
        parameter += 1 if depth == 0
      end
    end
    TemplateSignatureListener.new(parameter, @response_builder, global_state, node_context,
      dispatcher, sorbet_level)
  end
end
RubyLsp::Requests::SignatureHelp.prepend(TemplateSignatureRequest)
