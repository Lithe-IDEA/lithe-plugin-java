//go:build lithejsp

// This file belongs to the Java Support plugin repository. scripts/grammar/host-jsp.mjs
// adds it to a Lithe host checkout through a Go overlay, so the plugin's JSP grammar is
// checked by the host's own TextMate tokenizer, with the host's bundled HTML, CSS,
// JavaScript and Java grammars it includes. Nothing of the plugin is copied into the host.
package syntax

import (
	"os"
	"strings"
	"testing"

	"github.com/1lck/Lithe-IDEA/desktop/internal/editor/textmate"
)

// scopeCase names a text on a line and what the scopes of the token it starts in must and
// must not contain.
type scopeCase struct {
	line, text string
	nth        int
	want, not  []string
}

func TestPluginJSPGrammarOnHostTokenizer(t *testing.T) {
	path := os.Getenv("LITHE_JSP_GRAMMAR")
	if path == "" {
		t.Fatal("LITHE_JSP_GRAMMAR is not set; run scripts/grammar/host-jsp.mjs")
	}
	grammar, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	r := NewRegistry()
	jsp := PluginLanguage{ID: "jsp", Extensions: []string{"jsp", "jspf", "tag", "tagf"}, Grammar: grammar}
	if failed, _ := r.SetPluginLanguages([]PluginLanguage{jsp}); len(failed) != 0 {
		t.Fatalf("the host refused the JSP grammar: %v", failed)
	}
	if language, ok := r.ForPath("WEB-INF/views/index.jsp"); !ok || language.ID != "jsp" {
		t.Fatalf("index.jsp opens as %+v, %v", language, ok)
	}
	s := r.Syntax("jsp")
	if s == nil {
		t.Fatal("no syntax for the JSP language")
	}
	source := []string{
		`<%@ page contentType="text/html;charset=UTF-8" %>`,
		`<% int n = 0; // counted %><p>${user.name}</p>`,
		`<a href="<%= url %>" title="${title}">x</a>`,
		`<c:forEach items="${items}" var="item">`,
		`<script>var label = '${js}'; var more = a > b;</script>`,
		`<style>.a { color: red }</style>`,
		`<%-- ${hidden} <% ignored %> --%>`,
		`<p>\${literal} #{bean.value}</p>`,
		`<% if (ok) { %>`,
		`<b>yes</b>`,
		`<% } %>`,
		`<% /* left open %>`,
		`<i>after</i>`,
		`<jsp:include page="/footer.jsp"/>`,
	}
	cases := []scopeCase{
		{line: source[0], text: "page", want: []string{"meta.directive.jsp", "keyword.control.directive.jsp"}},
		{line: source[0], text: "contentType", want: []string{"entity.other.attribute-name.jsp"}},
		{line: source[1], text: "int", want: []string{"source.java.embedded.jsp", "storage.type.primitive.java"}},
		{line: source[1], text: "// counted", want: []string{"comment.line.double-slash.java"}},
		{line: source[1], text: "%>", want: []string{"punctuation.section.embedded.end.jsp"}, not: []string{"comment"}},
		{line: source[1], text: "p>", want: []string{"entity.name.tag"}, not: []string{"source.java"}},
		{line: source[1], text: "name", want: []string{"meta.expression.el.jsp", "variable.other.el.jsp"}},
		{line: source[2], text: "url", want: []string{"string.quoted", "meta.embedded.block.jsp"}},
		{line: source[2], text: "title}", want: []string{"string.quoted", "meta.expression.el.jsp"}},
		{line: source[3], text: "c", want: []string{"entity.name.tag.namespace.jsp"}, not: []string{"invalid"}},
		{line: source[3], text: "forEach", want: []string{"entity.name.tag.jsp"}, not: []string{"invalid"}},
		{line: source[3], text: "items}", want: []string{"meta.expression.el.jsp"}},
		{line: source[4], text: "var", want: []string{"source.js"}},
		{line: source[4], text: "js}", want: []string{"source.js", "meta.expression.el.jsp"}},
		{line: source[5], text: "color", want: []string{"source.css", "support.type.property-name"}},
		{line: source[6], text: "hidden", want: []string{"comment.block.jsp"}, not: []string{"meta.expression.el.jsp"}},
		{line: source[6], text: "ignored", want: []string{"comment.block.jsp"}, not: []string{"meta.embedded"}},
		{line: source[7], text: "literal", not: []string{"meta.expression.el.jsp"}},
		{line: source[7], text: "bean", want: []string{"meta.expression.el.jsp"}},
		{line: source[9], text: "b>", want: []string{"entity.name.tag"}, not: []string{"source.java"}},
		{line: source[10], text: "}", want: []string{"source.java.embedded.jsp"}},
		{line: source[12], text: "i>", want: []string{"entity.name.tag"}, not: []string{"comment", "source.java"}},
		{line: source[13], text: "jsp", want: []string{"entity.name.tag.namespace.jsp"}, not: []string{"invalid"}},
		{line: source[13], text: "include", want: []string{"entity.name.tag.jsp"}},
	}
	tokens := tokenizeLines(t, s.grammar, source)
	for _, c := range cases {
		index := -1
		for i, line := range source {
			if line == c.line {
				index = i
				break
			}
		}
		offset := strings.Index(c.line, c.text)
		if index < 0 || offset < 0 {
			t.Fatalf("case %q not found in %q", c.text, c.line)
		}
		scopes := scopesAt(tokens[index], offset)
		for _, want := range c.want {
			if !strings.Contains(scopes, want) {
				t.Errorf("%q in %q: scopes %q lack %q", c.text, c.line, scopes, want)
			}
		}
		for _, not := range c.not {
			if strings.Contains(scopes, not) {
				t.Errorf("%q in %q: scopes %q must not contain %q", c.text, c.line, scopes, not)
			}
		}
	}
	for i, line := range tokens {
		for _, token := range line {
			if scopes := token.Scopes.String(); strings.Contains(scopes, "invalid") {
				t.Errorf("line %q: a token is marked invalid: %s", source[i], scopes)
			}
		}
	}
}

func tokenizeLines(t *testing.T, grammar *textmate.Grammar, lines []string) [][]textmate.Token {
	t.Helper()
	tokenizer := grammar.NewTokenizer()
	var state *textmate.StateStack
	out := make([][]textmate.Token, len(lines))
	for i, line := range lines {
		var tokens []textmate.Token
		var stopped bool
		tokens, state, stopped = tokenizer.TokenizeLine(nil, []byte(line), state, 0)
		if stopped {
			t.Fatalf("tokenizing %q stopped", line)
		}
		out[i] = append([]textmate.Token(nil), tokens...)
	}
	return out
}

// scopesAt returns the scopes of the token that the ASCII offset starts in.
func scopesAt(tokens []textmate.Token, offset int) string {
	scopes := ""
	for _, token := range tokens {
		if token.Start > offset {
			break
		}
		scopes = token.Scopes.String()
	}
	return scopes
}
