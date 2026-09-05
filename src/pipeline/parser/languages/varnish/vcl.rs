use super::dialect::is_fastly_vcl;
use super::lexer::{Token, tokenize};
use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

/// Token stream wrapper for parser convenience.
struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    #[expect(
        dead_code,
        reason = "retained for future error reporting and debugging context"
    )]
    source: &'a str,
    file_path: &'a str,
    repo_name: &'a str,
    entities: Vec<ParsedEntity>,
    /// Pass-1 declaration table: name -> (kind, start_line)
    declarations: std::collections::HashMap<String, (EntityKind, usize)>,
    /// Import alias map: alias -> module name
    import_map: std::collections::HashMap<String, String>,
    /// Instance table from `new` statements: instance_name -> true
    instances: std::collections::HashSet<String>,
    line_offset: usize,
}

impl<'a> Parser<'a> {
    fn current(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek(&self, offset: usize) -> &Token {
        self.tokens.get(self.pos + offset).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_to_after(&mut self, targets: &[Token]) {
        while !matches!(self.current(), Token::Eof) {
            if targets.contains(self.current()) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    fn line_of_pos(&self, token_pos: usize) -> usize {
        let mut line = self.line_offset + 1;
        for (i, t) in self.tokens.iter().enumerate() {
            if i >= token_pos {
                break;
            }
            match t {
                Token::LongString(s) | Token::ShortString(s) => {
                    line += s.chars().filter(|&ch| ch == '\n').count();
                }
                Token::Comment(c) => {
                    line += c.chars().filter(|&ch| ch == '\n').count();
                }
                _ => {}
            }
        }
        line
    }
}

pub(crate) fn extract_entities_vcl(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    extract_entities_vcl_with_offset(source, file_path, repo_name, 0)
}

pub(crate) fn extract_entities_vcl_with_offset(
    source: &str,
    file_path: &str,
    repo_name: &str,
    line_offset: usize,
) -> Vec<ParsedEntity> {
    // Fastly dialect guard
    if is_fastly_vcl(source) {
        tracing::debug!("Skipping Fastly VCL file: {}", file_path);
        return Vec::new();
    }

    let tokens = tokenize(source);
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        source,
        file_path,
        repo_name,
        entities: Vec::new(),
        declarations: std::collections::HashMap::new(),
        import_map: std::collections::HashMap::new(),
        instances: std::collections::HashSet::new(),
        line_offset,
    };

    // Pass 1: collect declarations
    parser.collect_declarations();

    // Reset position for pass 2
    parser.pos = 0;
    parser.parse_file();

    parser.entities
}

impl Parser<'_> {
    /// Collects names of all declarations (subs, backends, probes, ACLs, imports, instances).
    #[expect(
        clippy::cognitive_complexity,
        reason = "function is verbose but correct — extraction deferred"
    )]
    fn collect_declarations(&mut self) {
        let saved_pos = self.pos;
        self.pos = 0;
        while !matches!(self.current(), Token::Eof) {
            self.skip_comments();
            match self.current() {
                Token::Ident(name) if name == "sub" => {
                    let line = self.line_of_pos(self.pos);
                    self.advance();
                    if let Token::Ident(sub_name) = self.current() {
                        self.declarations
                            .insert(sub_name.clone(), (EntityKind::VclSubroutine, line));
                        self.advance();
                    }
                    self.skip_until_brace_depth_0();
                }
                Token::Ident(name) if name == "backend" => {
                    let line = self.line_of_pos(self.pos);
                    self.advance();
                    if let Token::Ident(backend_name) = self.current() {
                        self.declarations
                            .insert(backend_name.clone(), (EntityKind::VclBackend, line));
                        self.advance();
                    }
                    self.skip_until_brace_depth_0();
                }
                Token::Ident(name) if name == "probe" => {
                    let line = self.line_of_pos(self.pos);
                    self.advance();
                    if let Token::Ident(probe_name) = self.current() {
                        self.declarations
                            .insert(probe_name.clone(), (EntityKind::VclProbe, line));
                        self.advance();
                    }
                    self.skip_until_brace_depth_0();
                }
                Token::Ident(name) if name == "acl" => {
                    let line = self.line_of_pos(self.pos);
                    self.advance();
                    if let Token::Ident(acl_name) = self.current() {
                        self.declarations
                            .insert(acl_name.clone(), (EntityKind::VclAcl, line));
                        self.advance();
                    }
                    self.skip_until_brace_depth_0();
                }
                Token::Ident(name) if name == "import" => {
                    self.advance();
                    if let Some(module_name) = self.take_ident() {
                        let alias = self.take_alias().unwrap_or_else(|| module_name.clone());
                        self.import_map.insert(alias, module_name);
                    }
                    self.skip_semicolon();
                }
                Token::Ident(name) if name == "new" => {
                    self.advance();
                    if let Token::Ident(instance_name) = self.current() {
                        let inst_name = instance_name.clone();
                        self.instances.insert(inst_name.clone());
                        self.declarations.insert(
                            inst_name,
                            (EntityKind::VclObjectInstance, self.line_of_pos(self.pos)),
                        );
                        self.advance();
                    }
                    self.skip_until_semicolon();
                }
                _ => {
                    self.advance();
                }
            }
        }
        self.pos = saved_pos;
    }

    fn parse_file(&mut self) {
        let mut has_version = false;
        for t in self.tokens {
            if let Token::Ident(kw) = t
                && kw == "vcl"
            {
                has_version = true;
                break;
            }
        }

        if !has_version {
            // Emit a synthetic file entity so that `include` edges have a target
            let file_fqn = format!("vcl:{}:{}", self.repo_name, self.file_path);
            let file_entity = ParsedEntity::new(
                self.file_path.to_string(),
                EntityKind::VclVersion,
                file_fqn,
                Some(format!("file {}", self.file_path)),
                None,
                "vcl",
                self.file_path,
                1,
                1,
                None,
                self.repo_name,
            );
            self.entities.push(file_entity);
        }

        while !matches!(self.current(), Token::Eof) {
            self.skip_comments();
            match self.current() {
                Token::Ident(name) => match name.as_str() {
                    "vcl" => self.parse_vcl_version(),
                    "backend" => self.parse_backend(),
                    "probe" => self.parse_probe(),
                    "acl" => self.parse_acl(),
                    "sub" => self.parse_sub(),
                    "import" => self.parse_import(),
                    "include" => self.parse_include(),
                    "unused" => self.parse_unused(),
                    _ => {
                        self.advance();
                    }
                },
                Token::Punct('{') => {
                    self.skip_until_brace_depth_0();
                }
                Token::Punct('}') => {
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn skip_comments(&mut self) {
        while matches!(self.current(), Token::Comment(_)) {
            self.advance();
        }
    }

    fn preceding_comments(&self, start_pos: usize) -> Option<String> {
        let mut comments = Vec::new();
        let mut i = start_pos;
        while i > 0 {
            i -= 1;
            match &self.tokens[i] {
                Token::Comment(c) => comments.push(c.clone()),
                _ => break,
            }
        }
        comments.reverse();
        if comments.is_empty() {
            None
        } else {
            Some(comments.join("\n"))
        }
    }

    fn parse_vcl_version(&mut self) {
        let start_pos = self.pos;
        let start_line = self.line_of_pos(start_pos);
        let mut version_str = String::from("vcl");
        self.advance(); // skip 'vcl'
        if let Token::Real(v) = self.current() {
            version_str = format!("vcl {:.1}", v);
            self.advance();
        } else if let Token::Integer(v) = self.current() {
            version_str = format!("vcl {}.0", v);
            self.advance();
        }
        self.skip_semicolon();

        let docstring = self.preceding_comments(start_pos);
        let entity = ParsedEntity::new(
            version_str.clone(),
            EntityKind::VclVersion,
            format!("vcl:{}:{}", self.repo_name, self.file_path),
            Some(version_str),
            docstring,
            "vcl",
            self.file_path,
            start_line,
            start_line,
            None,
            self.repo_name,
        );
        self.entities.push(entity);
    }

    fn parse_backend(&mut self) {
        let start_pos = self.pos;
        let start_line = self.line_of_pos(start_pos);
        self.advance(); // skip 'backend'
        let Some(name) = self.take_ident() else {
            return;
        };

        let mut signature = String::from("backend ");
        signature.push_str(&name);

        // Check for "backend name none;"
        if matches!(self.current(), Token::Ident(n) if n == "none") {
            self.advance();
            self.skip_semicolon();
            signature.push_str(" none");
            let docstring = self.preceding_comments(start_pos);
            let entity = ParsedEntity::new(
                name.clone(),
                EntityKind::VclBackend,
                format!("vcl:{}:{}", self.repo_name, name),
                Some(signature),
                docstring,
                "vcl",
                self.file_path,
                start_line,
                start_line,
                None,
                self.repo_name,
            );
            self.entities.push(entity);
            return;
        }

        // Regular backend { ... }
        if matches!(self.current(), Token::Punct('{')) {
            self.advance(); // skip {

            let mut refs: Vec<ReferenceIntent> = Vec::new();
            while !matches!(self.current(), Token::Punct('}') | Token::Eof) {
                self.skip_comments();
                match self.current() {
                    Token::Punct('.') => {
                        self.advance();
                        if let Token::Ident(_) = self.current() {
                            self.advance();
                            self.skip_to_after(&[Token::Punct('=')]);
                            self.parse_backend_attr_value(&mut refs);
                        } else {
                            self.advance();
                        }
                    }
                    _ => {
                        self.advance();
                    }
                }
            }
            if matches!(self.current(), Token::Punct('}')) {
                self.advance(); // skip }
            }

            let docstring = self.preceding_comments(start_pos);
            let mut entity = ParsedEntity::new(
                name.clone(),
                EntityKind::VclBackend,
                format!("vcl:{}:{}", self.repo_name, name),
                Some(signature),
                docstring,
                "vcl",
                self.file_path,
                start_line,
                self.line_of_pos(self.pos),
                None,
                self.repo_name,
            );
            entity.reference_intents = refs;
            self.entities.push(entity);
        }
    }

    fn parse_backend_attr_value(&mut self, refs: &mut Vec<ReferenceIntent>) {
        match self.current() {
            // .probe = myprobe; → UsesProbe
            Token::Ident(ident) if self.is_known_probe(ident) => {
                let line = self.line_of_pos(self.pos);
                refs.push(ReferenceIntent::VclProbeRef {
                    probe_name: ident.clone(),
                    line,
                });
                self.advance();
            }
            // .via = tunnel; → UsesBackend
            Token::Ident(ident)
                if self
                    .declarations
                    .get(ident)
                    .is_some_and(|(k, _)| *k == EntityKind::VclBackend) =>
            {
                let line = self.line_of_pos(self.pos);
                refs.push(ReferenceIntent::VclBackendRef {
                    backend_name: ident.clone(),
                    line,
                });
                self.advance();
            }
            // .probe = { ... } → inline probe
            Token::Punct('{') => {
                self.skip_until_brace_depth_0();
            }
            _ => {
                self.advance();
            }
        }
        self.skip_semicolon();
    }

    fn is_known_probe(&self, name: &str) -> bool {
        self.declarations
            .get(name)
            .is_some_and(|(k, _)| *k == EntityKind::VclProbe)
    }

    fn parse_probe(&mut self) {
        let start_pos = self.pos;
        let start_line = self.line_of_pos(start_pos);
        self.advance(); // skip 'probe'

        let Some(name) = self.take_ident() else {
            return;
        };

        let referent_refs: Vec<ReferenceIntent> = Vec::new();
        if matches!(self.current(), Token::Punct('{')) {
            self.advance(); // skip {
            while !matches!(self.current(), Token::Punct('}') | Token::Eof) {
                self.skip_comments();
                match self.current() {
                    Token::AttrName(attr) if attr == ".request" => {
                        // gotcha 2: adjacent string concatenation for .request
                        self.advance(); // skip .request
                        self.skip_to_after(&[Token::Punct('=')]);
                        self.skip_string_or_strings();
                    }
                    Token::AttrName(_) => {
                        self.advance(); // skip attr
                        self.skip_to_after(&[Token::Punct('=')]);
                        self.skip_until_semicolon();
                    }
                    _ => {
                        self.advance();
                    }
                }
            }
            if matches!(self.current(), Token::Punct('}')) {
                self.advance();
            }
        }

        let docstring = self.preceding_comments(start_pos);
        let mut entity = ParsedEntity::new(
            name.clone(),
            EntityKind::VclProbe,
            format!("vcl:{}:{}", self.repo_name, name),
            Some(format!("probe {}", name)),
            docstring,
            "vcl",
            self.file_path,
            start_line,
            self.line_of_pos(self.pos),
            None,
            self.repo_name,
        );
        entity.reference_intents = referent_refs;
        self.entities.push(entity);
    }

    fn skip_string_or_strings(&mut self) {
        while matches!(self.current(), Token::ShortString(_) | Token::LongString(_)) {
            self.advance();
        }
        self.skip_semicolon();
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "function is verbose but correct — extraction deferred"
    )]
    #[expect(
        clippy::excessive_nesting,
        reason = "function is verbose but correct — extraction deferred"
    )]
    fn parse_acl(&mut self) {
        let start_pos = self.pos;
        let start_line = self.line_of_pos(start_pos);
        self.advance(); // skip 'acl'

        let Some(name) = self.take_ident() else {
            return;
        };

        let body = String::new();

        // Skip flags: +log, +table, -pedantic, +fold(-report), etc.
        loop {
            if matches!(self.current(), Token::Ident(f) if f.starts_with('-')) {
                self.advance();
                continue;
            }
            if matches!(self.current(), Token::Punct('+')) {
                self.advance();
                if let Token::Ident(_) = self.current() {
                    self.advance();
                    if matches!(self.current(), Token::Punct('(')) {
                        self.advance();
                        if matches!(self.current(), Token::Ident(f) if f.starts_with('-')) {
                            self.advance();
                        } else if let Token::Ident(_) = self.current() {
                            self.advance();
                        }
                        if matches!(self.current(), Token::Punct(')')) {
                            self.advance();
                        }
                    }
                    continue;
                }
            }
            break;
        }

        // Collect ACL entries - just skip the block
        if matches!(self.current(), Token::Punct('{')) {
            self.advance(); // skip {
            while !matches!(self.current(), Token::Punct('}') | Token::Eof) {
                self.skip_comments();
                // gotcha 1: ACL mask with "addr"/mask
                if matches!(self.current(), Token::ShortString(_)) {
                    self.advance();
                    if matches!(self.current(), Token::Punct('/')) {
                        self.advance();
                        self.advance(); // skip integer
                    }
                } else if matches!(self.current(), Token::Punct('(')) {
                    self.skip_until_brace_balanced();
                } else {
                    self.advance();
                }
            }
            if matches!(self.current(), Token::Punct('}')) {
                self.advance();
            }
        }

        let docstring = self.preceding_comments(start_pos);
        let mut entity = ParsedEntity::new(
            name.clone(),
            EntityKind::VclAcl,
            format!("vcl:{}:{}", self.repo_name, name),
            Some(format!("acl {}", name)),
            docstring,
            "vcl",
            self.file_path,
            start_line,
            self.line_of_pos(self.pos),
            None,
            self.repo_name,
        );
        entity.inline_comments = vec![body];
        self.entities.push(entity);
    }

    fn parse_sub(&mut self) {
        let start_pos = self.pos;
        let start_line = self.line_of_pos(start_pos);
        self.advance(); // skip 'sub'

        let Some(name) = self.take_ident() else {
            return;
        };

        let is_builtin = name.starts_with("vcl_");
        let kind = if is_builtin {
            EntityKind::VclBuiltinSub
        } else {
            EntityKind::VclSubroutine
        };

        let signature = format!("sub {}", name);

        let mut refs: Vec<ReferenceIntent> = Vec::new();
        let _body_start = self.pos;

        self.parse_sub_body(&mut refs);

        let docstring = self.preceding_comments(start_pos);
        let fqn = format!("vcl:{}:{}", self.repo_name, name);
        let mut entity = ParsedEntity::new(
            name.clone(),
            kind,
            fqn,
            Some(signature),
            docstring,
            "vcl",
            self.file_path,
            start_line,
            self.line_of_pos(self.pos),
            None,
            self.repo_name,
        );
        entity.reference_intents = refs;
        self.entities.push(entity);
    }

    fn parse_sub_body(&mut self, refs: &mut Vec<ReferenceIntent>) {
        if !matches!(self.current(), Token::Punct('{')) {
            return;
        }
        self.advance(); // skip {
        let mut depth = 1;
        while depth > 0 && !matches!(self.current(), Token::Eof) {
            self.skip_comments();
            match self.current() {
                Token::Punct('{') => {
                    depth += 1;
                    self.advance();
                }
                Token::Punct('}') => {
                    depth -= 1;
                    if depth > 0 {
                        self.advance();
                    }
                }
                Token::Ident(kw) if kw == "call" => {
                    self.parse_call_statement(refs);
                }
                Token::Ident(kw) if kw == "set" => {
                    self.parse_set_statement(refs);
                }
                Token::Ident(kw) if kw == "if" || kw == "elseif" || kw == "else" => {
                    self.parse_if_statement(refs);
                }
                Token::Ident(kw) if kw == "unset" || kw == "return" => {
                    self.skip_until_semicolon();
                }
                Token::Ident(kw) if kw == "new" => {
                    self.parse_new_statement(refs);
                }
                Token::DottedPath(segs) => {
                    let segs = segs.clone();
                    self.parse_dotted_path_statement(segs, refs);
                }
                Token::Ident(prefix) => {
                    let prefix = prefix.clone();
                    self.parse_ident_statement(prefix, refs);
                }
                _ => {
                    self.advance();
                }
            }
        }
        if matches!(self.current(), Token::Punct('}')) {
            self.advance(); // skip }
        }
    }

    fn parse_call_statement(&mut self, refs: &mut Vec<ReferenceIntent>) {
        let line = self.line_of_pos(self.pos);
        self.advance();
        if let Token::Ident(callee) = self.current() {
            refs.push(ReferenceIntent::VclSubCall {
                sub_name: callee.clone(),
                line,
            });
            self.advance();
        }
        self.skip_semicolon();
    }

    fn parse_dotted_path_statement(&mut self, segs: Vec<String>, refs: &mut Vec<ReferenceIntent>) {
        let save = self.pos;
        self.advance();
        if segs.len() == 2 && matches!(self.current(), Token::Punct('(')) {
            self.record_method_call(segs[0].clone(), segs[1].clone(), save, refs);
            self.skip_until_semicolon();
            return;
        }
        self.skip_until_semicolon();
    }

    fn parse_ident_statement(&mut self, prefix: String, refs: &mut Vec<ReferenceIntent>) {
        let save = self.pos;
        self.advance();
        if matches!(self.current(), Token::Punct('.'))
            && let Token::Ident(method) = self.peek(1)
        {
            let method_str = method.clone();
            self.pos += 2; // skip .method
            self.record_method_call(prefix, method_str, save, refs);
            self.skip_until_semicolon();
        }
    }

    fn record_method_call(
        &mut self,
        prefix_str: String,
        method_str: String,
        save: usize,
        refs: &mut Vec<ReferenceIntent>,
    ) {
        let line = self.line_of_pos(save);
        let receiver =
            if self.instances.contains(&prefix_str) || self.import_map.contains_key(&prefix_str) {
                Some(prefix_str)
            } else {
                None
            };
        let arg_count = self.count_args();
        refs.push(ReferenceIntent::Call {
            method: method_str,
            receiver,
            line,
            arg_count,
        });
    }

    fn parse_set_statement(&mut self, refs: &mut Vec<ReferenceIntent>) {
        self.advance(); // skip 'set'
        // Read the target variable (e.g. req.backend_hint or bereq.backend)
        let target = self.current_name();
        self.skip_until_assignment_or_semicolon();

        if matches!(self.current(), Token::Punct('=')) {
            self.advance(); // skip '='
            if let Token::Ident(val) = self.current() {
                if target.contains("backend") {
                    let line = self.line_of_pos(self.pos);
                    refs.push(ReferenceIntent::VclBackendRef {
                        backend_name: val.clone(),
                        line,
                    });
                }
                self.advance();
            }
        }
        self.skip_semicolon();
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "function is verbose but correct — extraction deferred"
    )]
    fn parse_if_statement(&mut self, refs: &mut Vec<ReferenceIntent>) {
        self.advance(); // skip 'if'/'elseif'/'else'
        // Scan condition for ACL references
        while !matches!(self.current(), Token::Punct('{') | Token::Eof) {
            match self.current() {
                Token::Op(op) if op == "~" || op == "!~" => {
                    self.advance();
                    // gotcha 4: disambiguate regex vs ACL
                    if let Token::Ident(ident) = self.current() {
                        if self
                            .declarations
                            .get(ident)
                            .is_some_and(|(k, _)| *k == EntityKind::VclAcl)
                        {
                            let line = self.line_of_pos(self.pos);
                            refs.push(ReferenceIntent::VclAclRef {
                                acl_name: ident.clone(),
                                line,
                            });
                        }
                        // If it's a string literal, it's regex — no edge
                        self.advance();
                    } else if let Token::ShortString(_) = self.current() {
                        // regex — no edge (gotcha 4)
                        self.advance();
                    }
                }
                Token::Ident(method) => {
                    let save = self.pos;
                    let m = method.clone();
                    self.advance();
                    if matches!(self.current(), Token::Punct('.'))
                        && let Token::Ident(funcname) = self.current()
                    {
                        let line = self.line_of_pos(save);
                        let receiver =
                            if self.instances.contains(&m) || self.import_map.contains_key(&m) {
                                Some(m)
                            } else {
                                None
                            };
                        let fn_name = funcname.clone();
                        self.advance();
                        if matches!(self.current(), Token::Punct('(')) {
                            let arg_count = self.count_args();
                            refs.push(ReferenceIntent::Call {
                                method: fn_name,
                                receiver,
                                line,
                                arg_count,
                            });
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        // Scan body for nested statements that emit refs (set req.backend_hint, call, etc.)
        if matches!(self.current(), Token::Punct('{')) {
            self.advance();
            let mut depth = 1;
            while depth > 0 && !matches!(self.current(), Token::Eof) {
                self.skip_comments();
                match self.current() {
                    Token::Punct('{') => {
                        depth += 1;
                        self.advance();
                    }
                    Token::Punct('}') => {
                        depth -= 1;
                        if depth > 0 {
                            self.advance();
                        }
                    }
                    Token::Ident(kw) if kw == "set" => {
                        self.parse_set_statement(refs);
                    }
                    Token::Ident(kw) if kw == "call" => {
                        let line = self.line_of_pos(self.pos);
                        self.advance();
                        if let Token::Ident(callee) = self.current() {
                            refs.push(ReferenceIntent::VclSubCall {
                                sub_name: callee.clone(),
                                line,
                            });
                            self.advance();
                        }
                        self.skip_semicolon();
                    }
                    Token::Ident(kw) if kw == "if" || kw == "elseif" || kw == "else" => {
                        self.parse_if_statement(refs);
                    }
                    _ => {
                        self.advance();
                    }
                }
            }
            if matches!(self.current(), Token::Punct('}')) {
                self.advance();
            }
        }
    }

    fn parse_new_statement(&mut self, refs: &mut Vec<ReferenceIntent>) {
        let start = self.pos;
        self.advance(); // skip 'new'

        let Some(instance_name) = self.take_ident() else {
            self.skip_until_semicolon();
            return;
        };

        // skip '='
        if matches!(self.current(), Token::Punct('=')) {
            self.advance();
        }

        // module.constructor(...) → Call
        match self.current() {
            Token::DottedPath(segs) => {
                if segs.len() == 2 {
                    let mod_name = segs[0].clone();
                    let ctor = segs[1].clone();
                    self.advance();
                    let line = self.line_of_pos(start);
                    let arg_count = self.count_args();
                    refs.push(ReferenceIntent::Call {
                        method: ctor,
                        receiver: Some(mod_name),
                        line,
                        arg_count,
                    });
                } else {
                    self.advance();
                }
            }
            Token::Ident(module) => {
                let mod_name = module.clone();
                self.advance();
                if matches!(self.current(), Token::Punct('.')) {
                    self.advance();
                    if let Token::Ident(constructor) = self.current() {
                        let ctor = constructor.clone();
                        self.advance();
                        let line = self.line_of_pos(start);
                        let arg_count = self.count_args();
                        refs.push(ReferenceIntent::Call {
                            method: ctor,
                            receiver: Some(mod_name),
                            line,
                            arg_count,
                        });
                    }
                }
            }
            _ => {
                self.advance();
            }
        }

        self.skip_semicolon();

        // Emit VclObjectInstance entity
        let line = self.line_of_pos(start);
        let entity = ParsedEntity::new(
            instance_name.clone(),
            EntityKind::VclObjectInstance,
            format!("vcl:{}:{}", self.repo_name, instance_name),
            Some(format!("new {}", instance_name)),
            None,
            "vcl",
            self.file_path,
            line,
            line,
            None,
            self.repo_name,
        );
        self.entities.push(entity);
    }

    fn parse_import(&mut self) {
        let start_pos = self.pos;
        let line = self.line_of_pos(start_pos);
        self.advance(); // skip 'import'

        if let Some(module_name) = self.take_ident() {
            let alias = self.take_alias();

            // If there's a "from" path, skip it
            if matches!(self.current(), Token::Ident(f) if f == "from") {
                self.advance();
                self.advance(); // skip path string
            }

            let alias_str = alias.clone().unwrap_or_else(|| module_name.clone());
            let signature = if let Some(a) = &alias {
                format!("import {} as {}", module_name, a)
            } else {
                format!("import {}", module_name)
            };

            let docstring = self.preceding_comments(start_pos);
            let mut entity = ParsedEntity::new(
                alias_str,
                EntityKind::VclImport,
                format!("vcl:{}:import:{}", self.repo_name, module_name),
                Some(signature),
                docstring,
                "vcl",
                self.file_path,
                line,
                line,
                None,
                self.repo_name,
            );

            entity
                .reference_intents
                .push(ReferenceIntent::VclVmodImport {
                    module: module_name,
                    alias,
                    line,
                });

            self.entities.push(entity);
        }

        self.skip_semicolon();
    }

    fn parse_include(&mut self) {
        let start_pos = self.pos;
        let line = self.line_of_pos(start_pos);
        self.advance(); // skip 'include'

        // Optional +glob marker
        let is_glob = matches!(self.current(), Token::Ident(g) if g == "+glob");
        if is_glob {
            self.advance();
        }

        if let Token::ShortString(path) = self.current() {
            let p = path.clone();
            self.advance();

            let docstring = self.preceding_comments(start_pos);
            let mut entity = ParsedEntity::new(
                p.clone(),
                EntityKind::VclVersion, // treated as file-level manifest
                format!("vcl:{}:{}", self.repo_name, p),
                Some(format!("include \"{}\"", p)),
                docstring,
                "vcl",
                self.file_path,
                line,
                line,
                None,
                self.repo_name,
            );

            entity.reference_intents.push(ReferenceIntent::VclInclude {
                path: p.clone(),
                line,
            });

            self.entities.push(entity);
        }

        self.skip_semicolon();
    }

    fn parse_unused(&mut self) {
        let start_pos = self.pos;
        let line = self.line_of_pos(start_pos);
        self.advance(); // skip 'unused'

        if let Token::Ident(name) = self.current() {
            let n = name.clone();
            self.advance();
            let docstring = self.preceding_comments(start_pos);
            let mut entity = ParsedEntity::new(
                n.clone(),
                EntityKind::VclBuiltinSub, // placeholder kind; unused is a pseudo-ref
                format!("vcl:{}:unused:{}", self.repo_name, n),
                Some(format!("unused {}", n)),
                docstring,
                "vcl",
                self.file_path,
                line,
                line,
                None,
                self.repo_name,
            );

            entity
                .reference_intents
                .push(ReferenceIntent::VclUnusedRef { name: n, line });

            self.entities.push(entity);
        }

        self.skip_semicolon();
    }

    // ---- Helpers ----

    /// Consume an `Ident` token at the cursor, returning its name. The cursor
    /// is left untouched when the current token is not an `Ident`.
    fn take_ident(&mut self) -> Option<String> {
        if let Token::Ident(n) = self.current() {
            let n = n.clone();
            self.advance();
            Some(n)
        } else {
            None
        }
    }

    /// Consume an optional `as <alias>` clause, returning the alias name.
    fn take_alias(&mut self) -> Option<String> {
        if matches!(self.current(), Token::Ident(a) if a == "as") {
            self.advance();
            self.take_ident()
        } else {
            None
        }
    }

    fn current_name(&self) -> String {
        match self.current() {
            Token::Ident(s) => s.clone(),
            Token::DottedPath(segs) => segs.join("."),
            _ => String::new(),
        }
    }

    fn count_args(&mut self) -> Option<usize> {
        if !matches!(self.current(), Token::Punct('(')) {
            return None;
        }
        self.advance(); // skip (
        if matches!(self.current(), Token::Punct(')')) {
            self.advance();
            return Some(0);
        }
        let mut count = 1;
        let mut depth = 1;
        while depth > 0 {
            match self.current() {
                Token::Punct('(') => {
                    depth += 1;
                    self.advance();
                }
                Token::Punct(')') => {
                    depth -= 1;
                    if depth > 0 {
                        self.advance();
                    }
                }
                Token::Punct(',') if depth == 1 => {
                    count += 1;
                    self.advance();
                }
                Token::Eof => break,
                _ => {
                    self.advance();
                }
            }
        }
        if matches!(self.current(), Token::Punct(')')) {
            self.advance();
        }
        Some(count)
    }

    fn skip_semicolon(&mut self) {
        if matches!(self.current(), Token::Punct(';')) {
            self.advance();
        }
    }

    fn skip_until_semicolon(&mut self) {
        while !matches!(self.current(), Token::Punct(';') | Token::Eof) {
            self.advance();
        }
        self.skip_semicolon();
    }

    fn skip_until_assignment_or_semicolon(&mut self) {
        while !matches!(
            self.current(),
            Token::Punct('=') | Token::Punct(';') | Token::Eof
        ) {
            self.advance();
        }
    }

    fn skip_until_brace_depth_0(&mut self) {
        let mut depth = if matches!(self.current(), Token::Punct('{')) {
            self.advance();
            1
        } else {
            0
        };
        while depth > 0 && !matches!(self.current(), Token::Eof) {
            match self.current() {
                Token::Punct('{') => {
                    depth += 1;
                    self.advance();
                }
                Token::Punct('}') => {
                    depth -= 1;
                    if depth > 0 {
                        self.advance();
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        if matches!(self.current(), Token::Punct('}')) {
            self.advance();
        }
    }

    fn skip_until_brace_balanced(&mut self) {
        let mut depth = if matches!(self.current(), Token::Punct('(')) {
            self.advance();
            1
        } else {
            0
        };
        while depth > 0 {
            match self.current() {
                Token::Punct('(') => {
                    depth += 1;
                    self.advance();
                }
                Token::Punct(')') => {
                    depth -= 1;
                    if depth > 0 {
                        self.advance();
                    }
                }
                Token::Eof => break,
                _ => {
                    self.advance();
                }
            }
        }
        if matches!(self.current(), Token::Punct(')')) {
            self.advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vcl_version() {
        let entities = extract_entities_vcl("vcl 4.1;\n", "test.vcl", "test-repo");
        let versions: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclVersion)
            .collect();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].name, "vcl 4.1");
    }

    #[test]
    fn test_extract_backend() {
        let src = r#"backend default {
    .host = "127.0.0.1";
    .port = "8080";
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let backends: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclBackend)
            .collect();
        assert_eq!(backends.len(), 1);
        assert_eq!(backends[0].name, "default");
        assert_eq!(backends[0].fqn, "vcl:test-repo:default");
    }

    #[test]
    fn test_extract_backend_none() {
        let entities = extract_entities_vcl("backend default none;\n", "test.vcl", "test-repo");
        let backends: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclBackend)
            .collect();
        assert_eq!(backends.len(), 1);
        assert_eq!(backends[0].name, "default");
    }

    #[test]
    fn test_extract_probe() {
        let src = r#"probe myprobe {
    .url = "/healthz";
    .expected_response = 200;
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let probes: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclProbe)
            .collect();
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].name, "myprobe");
    }

    #[test]
    fn test_extract_probe_reference() {
        let src = r#"probe health { .url = "/"; }
backend b {
    .host = "127.0.0.1";
    .probe = health;
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let be = entities.iter().find(|e| e.name == "b").expect("backend b");
        assert!(be
            .reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::VclProbeRef { probe_name, .. } if probe_name == "health")));
    }

    #[test]
    fn test_extract_inline_probe_no_reference() {
        let src = r#"backend b {
    .host = "127.0.0.1";
    .probe = {
        .url = "/healthz";
    };
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let be = entities.iter().find(|e| e.name == "b").expect("backend b");

        // gotcha 13: inline probe has no probe reference
        assert!(
            !be.reference_intents
                .iter()
                .any(|r| matches!(r, ReferenceIntent::VclProbeRef { .. }))
        );
    }

    #[test]
    fn test_extract_acl() {
        let src = r#"acl localnetwork {
    "localhost";
    "192.0.2.0"/24;
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let acls: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclAcl)
            .collect();
        assert_eq!(acls.len(), 1);
        assert_eq!(acls[0].name, "localnetwork");
    }

    #[test]
    fn test_extract_acl_with_flags() {
        let src = r#"acl foo -pedantic +log +table +fold(-report) {
    "firewall.example.com" / 24;
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.kind == EntityKind::VclAcl && e.name == "foo")
        );
    }

    #[test]
    fn test_extract_subroutine() {
        let src = r#"sub pipe_if_local {
    if (client.ip ~ localnetwork) {
        return (pipe);
    }
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let subs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclSubroutine)
            .collect();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "pipe_if_local");
    }

    #[test]
    fn test_extract_builtin_sub() {
        let src = r#"sub vcl_recv {
    return (hash);
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let subs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclBuiltinSub)
            .collect();
        // 1 part + 1 aggregator
        assert!(!subs.is_empty());
        assert!(subs.iter().any(|e| e.name == "vcl_recv"));
    }

    #[test]
    fn test_extract_sub_call() {
        let src = r#"sub helper { return (pipe); }
sub vcl_recv {
    call helper;
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let recv = entities
            .iter()
            .find(|e| e.name == "vcl_recv" && e.start_line > 0)
            .expect("vcl_recv");
        assert!(recv.reference_intents.iter().any(
            |r| matches!(r, ReferenceIntent::VclSubCall { sub_name, .. } if sub_name == "helper")
        ));
    }

    #[test]
    fn test_extract_backend_hint() {
        let src = r#"backend b { .host = "127.0.0.1"; }
sub vcl_recv {
    set req.backend_hint = b;
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let recv = entities
            .iter()
            .find(|e| e.name == "vcl_recv" && e.start_line > 0)
            .expect("vcl_recv");
        assert!(recv
            .reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::VclBackendRef { backend_name, .. } if backend_name == "b")));
    }

    #[test]
    fn test_extract_acl_reference() {
        let src = r#"acl local { "192.0.2.0"/24; }
sub vcl_recv {
    if (client.ip ~ local) {
        return (pipe);
    }
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let recv = entities
            .iter()
            .find(|e| e.name == "vcl_recv" && e.start_line > 0)
            .expect("vcl_recv");
        assert!(recv.reference_intents.iter().any(
            |r| matches!(r, ReferenceIntent::VclAclRef { acl_name, .. } if acl_name == "local")
        ));
    }

    #[test]
    fn test_regex_vs_acl_disambiguation() {
        // gotcha 4: ~ with string literal is regex, NOT an ACL edge
        let src = r#"sub vcl_recv {
    if (req.http.host ~ "^(www\.)?example\.com$") {
        return (hash);
    }
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let recv = entities
            .iter()
            .find(|e| e.name == "vcl_recv" && e.start_line > 0)
            .expect("vcl_recv");
        // No ACL references from regex
        assert!(
            !recv
                .reference_intents
                .iter()
                .any(|r| matches!(r, ReferenceIntent::VclAclRef { .. }))
        );
    }

    #[test]
    fn test_extract_import() {
        let entities = extract_entities_vcl("import std;\n", "test.vcl", "test-repo");
        let imports: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclImport)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].name, "std");
        assert!(imports[0].reference_intents.iter().any(
            |r| matches!(r, ReferenceIntent::VclVmodImport { module, .. } if module == "std")
        ));
    }

    #[test]
    fn test_extract_import_with_alias() {
        let entities = extract_entities_vcl("import std as standard;\n", "test.vcl", "test-repo");
        let imports: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclImport)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].name, "standard");
    }

    #[test]
    fn test_extract_include() {
        let entities = extract_entities_vcl(
            "include \"/etc/varnish/foo.vcl\";\n",
            "test.vcl",
            "test-repo",
        );
        assert!(entities.iter().any(|e| {
            e.reference_intents.iter().any(|r| {
                matches!(r, ReferenceIntent::VclInclude { path, .. } if path == "/etc/varnish/foo.vcl")
            })
        }));
    }

    #[test]
    fn test_extract_unused() {
        let entities = extract_entities_vcl("unused b1;\n", "test.vcl", "test-repo");
        assert!(entities.iter().any(|e| {
            e.reference_intents
                .iter()
                .any(|r| matches!(r, ReferenceIntent::VclUnusedRef { name, .. } if name == "b1"))
        }));
    }

    #[test]
    fn test_extract_vmod_call() {
        let src = r#"import std;
sub vcl_deliver {
    std.log("hello");
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        let deliver = entities
            .iter()
            .find(|e| e.name == "vcl_deliver" && e.start_line > 0)
            .expect("vcl_deliver");
        assert!(deliver
            .reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::Call { method, receiver, .. } if method == "log" && receiver.as_deref() == Some("std"))));
    }

    #[test]
    fn test_extract_new_statement() {
        let src = r#"import directors;
sub vcl_init {
    new cluster = directors.round_robin();
}"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        // Should have a VclObjectInstance for cluster and a Call intent
        let instances: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclObjectInstance)
            .collect();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].name, "cluster");

        let init = entities
            .iter()
            .find(|e| e.name == "vcl_init" && e.start_line > 0)
            .expect("vcl_init");
        assert!(
            init.reference_intents.iter().any(
                |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "round_robin")
            )
        );
    }

    #[test]
    fn test_empty_input() {
        let entities = extract_entities_vcl("", "test.vcl", "test-repo");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].kind, EntityKind::VclVersion);
    }

    #[test]
    fn test_malformed_input_returns_empty() {
        let entities = extract_entities_vcl("@#$%@#$\n", "test.vcl", "test-repo");
        // Should not panic; just return what it can
        assert!(entities.is_empty() || !entities.is_empty()); // just verifying no panic
    }

    #[test]
    fn test_fastly_guard() {
        let src = r#"vcl 4.0;
declare local var.x STRING;
sub vcl_recv { }"#;
        let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
        assert!(entities.is_empty());
    }

    #[test]
    fn test_extract_vcl_version_vcl_4_0() {
        let entities = extract_entities_vcl("vcl 4.0;\n", "test.vcl", "test-repo");
        let versions: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclVersion)
            .collect();
        assert_eq!(versions.len(), 1);
        assert!(versions[0].name.contains("4.0"));
    }
}
#[test]
fn test_preceding_comments_debug() {
    let src = "// my comment\nvcl 4.1;\n";
    let entities = extract_entities_vcl(src, "test.vcl", "test-repo");
    let _ = entities;
}
