//! The architecture as a graph: one node per service and per topic, and the arrows that the project implies.
//! Arrows are never stored; they come from the producers of the topics, the `Publish` and `CallRpc` steps, and the
//! consumers. No window here, so the rules can be tested.

use std::collections::{BTreeMap, BTreeSet};

use sb_model::{Consumer, Diagnostic, Project, ServiceKind, Severity, Step};

use super::mail::{add_mail_event, consumer_name};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Id {
    Service(usize),
    Topic(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tag {
    Rpc,
    Get,
    Post,
    OtherHttp,
    Kafka,
    Mail,
    Job,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub tag: Tag,
    pub label: &'static str,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: Id,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// `:50051`, `mta`, `sse`.
    pub badge: String,
    pub rows: Vec<Row>,
    /// Topics: the message, then the partitions.
    pub meta: Vec<String>,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeKind {
    Publish,
    Consume,
    Rpc,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: Id,
    pub to: Id,
    pub kind: EdgeKind,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

pub const HEADER: f64 = 34.0;
pub const ROW: f64 = 22.0;
const SERVICE_W: f64 = 220.0;
const TOPIC_W: f64 = 210.0;

impl Graph {
    pub fn node(&self, id: Id) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The node under a point of the canvas, the one drawn last (on top) first.
    pub fn hit(&self, x: f64, y: f64) -> Option<Id> {
        self.nodes
            .iter()
            .rev()
            .find(|n| x >= n.x && x <= n.x + n.w && y >= n.y && y <= n.y + n.h)
            .map(|n| n.id)
    }

    /// `(min x, min y, max x, max y)`, or nothing for an empty graph.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        self.nodes.iter().fold(None, |acc, n| {
            let b = (n.x, n.y, n.x + n.w, n.y + n.h);
            Some(acc.map_or(b, |a| {
                (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
            }))
        })
    }
}

fn count(diagnostics: &[Diagnostic], prefix: &str) -> (usize, usize) {
    let mine = diagnostics
        .iter()
        .filter(|d| d.path == prefix || d.path.starts_with(&format!("{prefix}/")));
    mine.fold((0, 0), |(e, w), d| match d.severity {
        Severity::Error => (e + 1, w),
        Severity::Warning => (e, w + 1),
    })
}

pub fn build(project: &Project, diagnostics: &[Diagnostic]) -> Graph {
    let mut g = Graph::default();
    let topic_index: BTreeMap<&str, usize> = project
        .topics
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), i))
        .collect();
    let service_index: BTreeMap<&str, usize> = project
        .services
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.as_str(), i))
        .collect();

    for (i, svc) in project.services.iter().enumerate() {
        let mut rows = vec![];
        for r in &svc.rpcs {
            rows.push(Row {
                tag: Tag::Rpc,
                label: "RPC",
                text: r.name.clone(),
            });
        }
        for r in &svc.routes {
            let (tag, label) = match r.method {
                sb_model::HttpMethod::Get => (Tag::Get, "GET"),
                sb_model::HttpMethod::Post => (Tag::Post, "POST"),
                sb_model::HttpMethod::Put => (Tag::OtherHttp, "PUT"),
                sb_model::HttpMethod::Patch => (Tag::OtherHttp, "PATCH"),
                sb_model::HttpMethod::Delete => (Tag::OtherHttp, "DELETE"),
            };
            rows.push(Row {
                tag,
                label,
                text: r.path.clone(),
            });
        }
        for c in &svc.consumers {
            let mail = c.mail.is_some();
            rows.push(Row {
                tag: if mail { Tag::Mail } else { Tag::Kafka },
                label: if mail { "MAIL" } else { "KAFKA" },
                text: if mail {
                    c.topic.clone()
                } else {
                    c.name.clone()
                },
            });
        }
        for j in &svc.jobs {
            rows.push(Row {
                tag: Tag::Job,
                label: "TÂCHE",
                text: j.name.clone(),
            });
        }
        let (errors, warnings) = count(diagnostics, &format!("/services/{i}"));
        g.nodes.push(Node {
            id: Id::Service(i),
            name: svc.name.clone(),
            x: svc.position.x,
            y: svc.position.y,
            w: SERVICE_W,
            h: HEADER + rows.len().max(1) as f64 * ROW + 14.0,
            badge: match svc.kind {
                ServiceKind::Mta => "mta".into(),
                ServiceKind::Realtime => "sse".into(),
                ServiceKind::Standard => format!(":{}", svc.port),
            },
            rows,
            meta: vec![],
            errors,
            warnings,
        });
    }

    for (i, t) in project.topics.iter().enumerate() {
        let (errors, warnings) = count(diagnostics, &format!("/topics/{i}"));
        let key = t
            .key_field
            .as_ref()
            .map_or(String::new(), |k| format!(" · clé {k}"));
        g.nodes.push(Node {
            id: Id::Topic(i),
            name: t.name.clone(),
            x: t.position.x,
            y: t.position.y,
            w: TOPIC_W,
            h: HEADER + 44.0,
            badge: String::new(),
            rows: vec![],
            meta: vec![
                t.message.clone(),
                format!(
                    "{} partition{}{key}",
                    t.partitions,
                    if t.partitions > 1 { "s" } else { "" }
                ),
            ],
            errors,
            warnings,
        });
    }

    for (i, svc) in project.services.iter().enumerate() {
        // Declared producers and publishing steps both draw the arrow (a step without a declaration is also an error).
        let mut published: BTreeSet<&str> = project
            .topics
            .iter()
            .filter(|t| t.producers.contains(&svc.name))
            .map(|t| t.name.as_str())
            .collect();
        let mut called: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let handlers = svc
            .rpcs
            .iter()
            .map(|r| &r.steps)
            .chain(svc.routes.iter().map(|r| &r.steps))
            .chain(svc.consumers.iter().map(|c| &c.steps))
            .chain(svc.jobs.iter().map(|j| &j.steps));
        for steps in handlers {
            for step in steps {
                step.walk(&mut |s| match s {
                    Step::Publish { topic, .. } if !topic.is_empty() => {
                        published.insert(topic);
                    }
                    Step::CallRpc {
                        service, method, ..
                    } if !service.is_empty() => {
                        called.entry(service).or_default().insert(method);
                    }
                    _ => {}
                });
            }
        }
        for topic in published {
            if let Some(&t) = topic_index.get(topic) {
                g.edges.push(Edge {
                    from: Id::Service(i),
                    to: Id::Topic(t),
                    kind: EdgeKind::Publish,
                    label: String::new(),
                });
            }
        }
        for c in &svc.consumers {
            if let Some(&t) = topic_index.get(c.topic.as_str()) {
                g.edges.push(Edge {
                    from: Id::Topic(t),
                    to: Id::Service(i),
                    kind: EdgeKind::Consume,
                    label: String::new(),
                });
            }
        }
        for (target, methods) in called {
            if let Some(&t) = service_index.get(target) {
                g.edges.push(Edge {
                    from: Id::Service(i),
                    to: Id::Service(t),
                    kind: EdgeKind::Rpc,
                    label: methods.into_iter().collect::<Vec<_>>().join(", "),
                });
            }
        }
    }
    g
}

/// What drawing an arrow from `from` to `to` does to the project: a service that publishes on a topic becomes one of
/// its producers; a service that listens to a topic gets a consumer (or, for a mail service, a mail).
pub fn connect(p: &mut Project, from: Id, to: Id) -> Result<String, String> {
    match (from, to) {
        (Id::Service(s), Id::Topic(t)) => {
            let name = p.services.get(s).map(|x| x.name.clone()).ok_or("service inconnu")?;
            let topic = p.topics.get_mut(t).ok_or("topic inconnu")?;
            if topic.producers.contains(&name) {
                return Err(format!("{name} publie déjà sur {}.", topic.name));
            }
            topic.producers.push(name.clone());
            Ok(format!("{name} publie maintenant sur {}.", topic.name))
        }
        (Id::Topic(t), Id::Service(s)) => {
            let topic = p.topics.get(t).map(|x| x.name.clone()).ok_or("topic inconnu")?;
            let svc = p.services.get(s).ok_or("service inconnu")?;
            if svc.consumers.iter().any(|c| c.topic == topic) {
                return Err(format!("{} écoute déjà {topic}.", svc.name));
            }
            let (name, kind) = (svc.name.clone(), svc.kind);
            if kind == ServiceKind::Mta {
                add_mail_event(p, s, &topic);
            } else if kind == ServiceKind::Standard {
                let taken: Vec<String> = svc.consumers.iter().map(|c| c.name.clone()).collect();
                p.services[s].consumers.push(Consumer {
                    name: consumer_name(&topic, &taken),
                    topic: topic.clone(),
                    group_id: None,
                    idempotent_on: None,
                    mail: None,
                    steps: vec![],
                });
            } else {
                return Err(format!("{name} est prêt à l'emploi : il n'écoute aucun topic."));
            }
            Ok(format!("{name} écoute maintenant {topic}."))
        }
        _ => Err("Tracez d'un service vers un topic (il y publie), ou d'un topic vers un service (il l'écoute).".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_draws_six_nodes_and_seven_arrows() {
        // The same numbers as the web editor's own test on this project.
        let g = build(&sb_model::sample::orders(), &[]);
        assert_eq!(g.nodes.len(), 6);
        assert_eq!(g.edges.len(), 7);
        assert!(g.edges.iter().any(|e| e.kind == EdgeKind::Publish));
        assert!(g.edges.iter().any(|e| e.kind == EdgeKind::Consume));
    }

    #[test]
    fn an_arrow_to_a_topic_that_does_not_exist_is_not_drawn() {
        let mut p = sb_model::sample::orders();
        let before = build(&p, &[]).edges.len();
        p.topics[0].name = "renamed.elsewhere".into();
        let after = build(&p, &[]).edges.len();
        assert!(after < before, "{after} < {before}");
    }

    #[test]
    fn the_top_most_node_is_hit_and_the_void_is_not() {
        let g = build(&sb_model::sample::orders(), &[]);
        let n = &g.nodes[0];
        assert_eq!(g.hit(n.x + 5.0, n.y + 5.0), Some(n.id));
        assert_eq!(g.hit(-5000.0, -5000.0), None);
        let (x0, y0, x1, y1) = g.bounds().unwrap();
        assert!(x1 > x0 && y1 > y0);
        assert!(Graph::default().bounds().is_none());
    }

    #[test]
    fn drawing_an_arrow_edits_the_project() {
        let mut p = sb_model::sample::orders();
        let svc = |p: &Project, n: &str| p.services.iter().position(|s| s.name == n).unwrap();
        let top = |p: &Project, n: &str| p.topics.iter().position(|t| t.name == n).unwrap();

        // Notifications starts to publish on orders.created.
        let (n, t) = (svc(&p, "Notifications"), top(&p, "orders.created"));
        let before = build(&p, &[]).edges.len();
        connect(&mut p, Id::Service(n), Id::Topic(t)).unwrap();
        assert_eq!(build(&p, &[]).edges.len(), before + 1);
        assert!(
            connect(&mut p, Id::Service(n), Id::Topic(t)).is_err(),
            "already done"
        );

        // Orders starts to listen to payments.failed.
        let (o, f) = (svc(&p, "Orders"), top(&p, "payments.failed"));
        connect(&mut p, Id::Topic(f), Id::Service(o)).unwrap();
        assert!(
            p.services[o]
                .consumers
                .iter()
                .any(|c| c.topic == "payments.failed")
        );
        assert!(connect(&mut p, Id::Topic(f), Id::Service(o)).is_err());

        // Two services, or two topics, are not connected by an arrow.
        assert!(connect(&mut p, Id::Service(o), Id::Service(n)).is_err());
        assert!(connect(&mut p, Id::Topic(t), Id::Topic(f)).is_err());
    }

    #[test]
    fn diagnostics_are_counted_on_their_node() {
        let d = |sev, path: &str| Diagnostic {
            severity: sev,
            code: "X".into(),
            message: String::new(),
            path: path.into(),
        };
        let g = build(
            &sb_model::sample::orders(),
            &[
                d(Severity::Error, "/services/0/rpcs/1"),
                d(Severity::Warning, "/services/0"),
                d(Severity::Error, "/services/10"),
            ],
        );
        assert_eq!((g.nodes[0].errors, g.nodes[0].warnings), (1, 1));
        assert_eq!((g.nodes[1].errors, g.nodes[1].warnings), (0, 0));
    }
}
