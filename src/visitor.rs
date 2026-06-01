use gix::{bstr::BString, traverse::tree::visit::Action as VisitAction};

pub struct TemplateVisitor {
    path_stack: Vec<BString>,
    templates: Vec<(String, gix::ObjectId)>,
}

impl Default for TemplateVisitor {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateVisitor {
    pub fn new() -> Self {
        Self {
            path_stack: vec![],
            templates: vec![],
        }
    }

    // move the template
    pub fn output(self) -> Vec<(String, gix::ObjectId)> {
        self.templates
    }
}

impl gix::traverse::tree::Visit for TemplateVisitor {
    fn pop_front_tracked_path_and_set_current(&mut self) {}
    fn pop_back_tracked_path_and_set_current(&mut self) {}
    fn push_back_tracked_path_component(&mut self, _component: &gix::bstr::BStr) {}

    fn push_path_component(&mut self, component: &gix::bstr::BStr) {
        self.path_stack.push(component.to_owned());
    }

    fn pop_path_component(&mut self) {
        self.path_stack.pop();
    }

    fn visit_tree(&mut self, _entry: &gix::objs::tree::EntryRef<'_>) -> VisitAction {
        VisitAction::Continue(true)
    }

    fn visit_nontree(&mut self, entry: &gix::objs::tree::EntryRef<'_>) -> VisitAction {
        let filename = entry.filename;
        if filename.ends_with(b".gitignore") {
            let name_str = self.path_stack
                .iter()
                .map(|p| {
                    let s = String::from_utf8_lossy(p);
                    s.strip_suffix(".gitignore").unwrap_or(&s).to_lowercase()
                })
                .collect::<Vec<_>>()
                .join("/");
            
            self.templates.push((name_str, entry.oid.to_owned()));
        }
        VisitAction::Continue(false)
    }
}
