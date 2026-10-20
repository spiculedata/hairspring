package hairspring

import (
	"encoding/json"
	"fmt"
	"hash/fnv"
	"strings"
)

// maxLineBytes is the protocol's cap on a whole line, prefix and newline included.
const maxLineBytes = 4096

const (
	linePrefix = "HAIRSPRING_EVENT "
	// numWidth right-aligns counters, so a report's length does not depend on their values.
	numWidth = 20
	// witnessWidth is the widest witness object (thread 64 bytes, ord and four values in 20 characters each);
	// a witness or null is padded to it for the same reason.
	witnessWidth = 197
)

func quote(s string) string {
	b, _ := json.Marshal(s) // a string always marshals; Declare already rejected invalid UTF-8
	return string(b)
}

func num[T ~uint64 | ~int64](n T) string { return fmt.Sprintf("%*d", numWidth, n) }

// line frames a JSON object body (everything after `{"v":1,"kind":"<kind>"`, without the closing brace).
func line(kind, body string) []byte {
	return []byte(linePrefix + `{"v":1,"kind":"` + kind + `"` + body + "}\n")
}

func helloLine(proc, hash string) []byte {
	return line("hello", `,"sdk":"go","sdk_version":"`+SDKVersion+`","proc":`+quote(proc)+
		`,"catalog_hash":"`+hash+`","signal_handlers":false`)
}

// catalogLine drops the message and location when escaping would push the line over the cap.
func catalogLine(proc string, s Site) []byte {
	head := `,"proc":` + quote(proc) + `,"id":` + quote(s.ID) + `,"type":"` + string(s.Kind) +
		fmt.Sprintf(`","must_hit":%t`, s.Kind.mustHit())
	var text string
	if s.Msg != "" {
		text += `,"msg":` + quote(s.Msg)
	}
	if s.File != "" {
		text += `,"loc":{"file":` + quote(s.File) + fmt.Sprintf(`,"line":%d`, s.Line)
		if s.Func != "" {
			text += `,"func":` + quote(s.Func)
		}
		text += "}"
	}
	if l := line("catalog", head+text); len(l) <= maxLineBytes {
		return l
	}
	return line("catalog", head)
}

func witnessJSON(w *Witness) string {
	text := "null"
	if w != nil {
		text = fmt.Sprintf(`{"thread":%s,"ord":%s,"vals":[%d,%d,%d,%d]}`, quote(w.Thread), num(w.Ord),
			w.Vals[0], w.Vals[1], w.Vals[2], w.Vals[3])
	}
	if pad := witnessWidth - len(text); pad > 0 {
		text += strings.Repeat(" ", pad)
	}
	return text
}

func assertLine(proc string, r SiteReport) []byte {
	body := `,"proc":` + quote(proc) + `,"id":` + quote(r.Site.ID) +
		`,"hits":` + num(r.Hits()) + `,"trues":` + num(r.Trues) + `,"falses":` + num(r.Falses)
	if !r.Site.Kind.isPoint() {
		body += `,"first_false":` + witnessJSON(r.FirstFalse)
	}
	return line("assert", body+`,"first_true":`+witnessJSON(r.FirstTrue))
}

func workLine(proc string, ops uint64) []byte {
	return line("work", `,"proc":`+quote(proc)+`,"ops":`+num(ops))
}

// endLine closes the report. threads is always 0: the SDK sends no thread lines.
func endLine(proc string, status int64, sites, evals, dropped uint64) []byte {
	return line("end", `,"proc":`+quote(proc)+`,"reason":"exit","status":`+num(status)+
		fmt.Sprintf(`,"sites":%d,"evals":`, sites)+num(evals)+`,"dropped":`+num(dropped)+`,"threads":0`)
}

// catalogHash is FNV-1a over the sorted (id, kind) pairs, so it changes when the catalog does.
func catalogHash(r Report) string {
	h := fnv.New64a()
	for _, s := range r.Sites {
		fmt.Fprintf(h, "%s\x00%s\n", s.Site.ID, s.Site.Kind)
	}
	return fmt.Sprintf("%016x", h.Sum64())
}

// violationLine is the legacy marker: one line, id and message (or the kind's meaning) and the count.
func violationLine(r SiteReport) []byte {
	reason, count := r.Site.Msg, fmt.Sprintf("%d false", r.Falses)
	if r.Site.Kind == KindUnreachable {
		reason, count = "unreachable site hit", fmt.Sprintf("%d hit", r.Trues)
	}
	if reason == "" {
		reason = "always-type property was false"
	}
	return []byte(fmt.Sprintf("HAIRSPRING_VIOLATION: %s: %s (%s)\n", r.Site.ID, reason, count))
}
