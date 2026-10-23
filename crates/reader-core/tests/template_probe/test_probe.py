"""Checks the experiment's public projection/source-edit boundary."""
import unittest
import os
import subprocess
from probe import Projection, project, apply_edit, apply_provider_edit, markup_project, apply_markup_edit


try:
    ERB_AVAILABLE = subprocess.run(['ruby', '-rerb', '-e', ''],
                                   capture_output=True).returncode == 0
except FileNotFoundError:
    ERB_AVAILABLE = False


class ProjectionTests(unittest.TestCase):
    @unittest.skipUnless(ERB_AVAILABLE, 'requires Ruby with the ERB scanner')
    def test_erb_automatic_regions_preserve_unicode_crlf_and_multiline(self):
        source = '<p>😀<%= "😀"; ReadyWidget.gre %></p>\r\n<% x =\r\n  1 %>'
        projection = project(source, 'erb')
        self.assertIn('"😀"; ReadyWidget.gre', projection.generated)
        self.assertIn('x =\r\n  1', projection.generated)
        start = projection.generated.index('gre')
        self.assertEqual(apply_edit(projection, start, start + 3, 'greet'),
                         source.replace('ReadyWidget.gre', 'ReadyWidget.greet'))

    @unittest.skipUnless(os.environ.get('EDITOR_HAML_GEM_HOME'), 'requires owned Haml 7.5.1 test gems')
    def test_haml_recognizes_indented_output_interpolation_and_pipe_multiline(self):
        source = '%div(title="😀")\r\n  = "😀"; ReadyWidget.gre\r\n  %p Hello #{ReadyWidget.gre}\r\n  = ReadyWidget.greet( |\r\n      "world"         |\r\n    )                 |\r\n'
        projection = project(source, 'haml')
        self.assertIn('"😀"; ReadyWidget.gre', projection.generated)
        self.assertIn('ReadyWidget.greet( \n', projection.generated)
        self.assertNotIn('|', projection.generated)
        start = projection.generated.index('ReadyWidget.gre', projection.generated.index('ReadyWidget.gre') + 1) + 12
        self.assertEqual(apply_edit(projection, start, start + 3, 'greet'),
                         source.replace('#{ReadyWidget.gre}', '#{ReadyWidget.greet}'))

    def test_provider_edit_refuses_stale_foreign_and_cross_region_transactions(self):
        projection = Projection('<%= ReadyWidget.gre %><%= other %>',
                                ' ReadyWidget.gre \n other \n', [(3, 20, 0, 17), (25, 32, 18, 25)])
        edit = {'textEdit': {'range': {'start': {'line': 0, 'character': 13},
                                     'end': {'line': 0, 'character': 16}}, 'newText': 'greet'}}
        self.assertEqual(apply_provider_edit(projection, edit, 'utf-32', 1, 'fixture'),
                         '<%= ReadyWidget.greet %><%= other %>')
        for version, uri in [(0, 'fixture'), (1, 'other')]:
            with self.assertRaises(ValueError):
                apply_provider_edit(projection, edit, 'utf-32', version, uri)
        edit['additionalTextEdits'] = [{'range': {}, 'newText': 'unsafe'}]
        with self.assertRaises(ValueError):
            apply_provider_edit(projection, edit, 'utf-32', 1, 'fixture')
        with self.assertRaises(ValueError):
            apply_edit(projection, 0, len(projection.generated), 'unsafe')

    def test_haml_markup_provider_edit_preserves_percent_and_attribute_parentheses(self):
        for source, generated, start, end, new_text, expected in [
            ('  %di', '  <di', 3, 5, 'div', '  %div'),
            ('  %div(cla)', '  <div cla>', 7, 10, 'class="$1"', '  %div(class="")'),
        ]:
            projection = markup_project(source, 'haml')
            self.assertEqual(projection.generated, generated)
            item = {'textEdit': {'range': {'start': {'line': 0, 'character': start},
                                         'end': {'line': 0, 'character': end}}, 'newText': new_text}}
            item['insertTextFormat'] = 2 if '$1' in new_text else 1
            self.assertEqual(apply_markup_edit(projection, item), expected)

    @unittest.skipUnless(os.environ.get('EDITOR_HAML_GEM_HOME'), 'requires owned Haml 7.5.1 test gems')
    def test_unsupported_haml_is_refused_instead_of_silently_losing_ruby(self):
        for source in ['%div(title=ReadyWidget.gre)', '%div(title="unfinished',
                       '- widgets.each do', ':ruby\n  ReadyWidget.gre',
                       '- if visible\n  = ReadyWidget.gre']:
            with self.subTest(source=source), self.assertRaises(ValueError):
                project(source, 'haml')

    @unittest.skipUnless(ERB_AVAILABLE, 'requires Ruby with the ERB scanner')
    def test_erb_scanner_excludes_comments_and_escaped_openers(self):
        projection = project('<%%= outside %><%# ignored %><%= ReadyWidget.gre %>', 'erb')
        self.assertEqual(projection.generated, ' ReadyWidget.gre \n')

    def test_provider_edit_refuses_half_surrogate_and_out_of_bounds_positions(self):
        projection = Projection('<%= "😀"; ReadyWidget.gre %>',
                                ' "😀"; ReadyWidget.gre \n', [(3, 25, 0, 22)])
        for line, column in [(0, 3), (100, 0), (0, 100), (-1, 0)]:
            item = {'textEdit': {'range': {'start': {'line': line, 'character': column},
                                          'end': {'line': line, 'character': column}}, 'newText': 'x'}}
            with self.subTest(line=line, column=column), self.assertRaises(ValueError):
                apply_provider_edit(projection, item, 'utf-16', 1, 'fixture')


if __name__ == '__main__':
    unittest.main()
