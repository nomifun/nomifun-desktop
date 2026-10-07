//! Worlds of one granted page, including its descendant frames. Element refs
//! remain observation-scoped; no model-supplied frame/session is accepted.
use super::*;
use crate::frame_geometry::{CHECK_FRAME_HIT, CHECK_FRAME_OWNER, ContentQuad};
use chromiumoxide::cdp::browser_protocol::dom::{
    GetBoxModelParams, GetFrameOwnerParams, ResolveNodeParams,
};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) struct Document {
    pub session: String,
    pub frame: String,
    pub loader: String,
    pub object: String,
    pub context: i64,
    pub group: String,
    parent: Option<usize>,
    owner: Option<String>,
    owner_group: String,
}

impl Document {
    pub fn root(state: &TabAutomation) -> Self {
        Self {
            session: state.session.clone(),
            frame: state.frame.clone(),
            loader: state.loader.clone(),
            object: state.object.clone(),
            context: state.context,
            group: state.group.clone(),
            parent: None,
            owner: None,
            owner_group: String::new(),
        }
    }
}

#[derive(Default)]
pub(super) struct Documents {
    pub routes: Option<super::frame_routes::FrameRoutes>,
    worlds: Vec<Document>,
    refs: BTreeMap<String, (usize, String)>,
    active: usize,
}

struct Plan {
    frame: String,
    loader: String,
    session: String,
    parent: Option<usize>,
    url: String,
}

fn plans(trees: &[(String, Value)], root: &str) -> Result<Vec<Plan>, Error> {
    let mut nodes = BTreeMap::<String, (String, String, Option<String>, String)>::new();
    let mut count = 0;
    for (session, tree) in trees {
        let mut pending = vec![(tree, None)];
        while let Some((tree, parent)) = pending.pop() {
            count += 1;
            if count > 256 {
                return Err(Error::ExecutionFailed);
            }
            let frame = &tree["frame"];
            let id = frame["id"].as_str().ok_or(Error::InvalidInput)?;
            let parent = frame["parentId"].as_str().or(parent).map(str::to_owned);
            let loader = frame["loaderId"].as_str().unwrap_or("").to_owned();
            let url = frame["url"].as_str().unwrap_or("").to_owned();
            if let Some(existing) = nodes.get(id) {
                if existing.2.is_some() && parent.is_some() && existing.2 != parent {
                    return Err(Error::InvalidInput);
                }
                // OOPIF trees contain the authoritative local document. Empty
                // parent placeholders must not override that document.
                if !loader.is_empty() && !existing.1.is_empty() && existing.1 != loader {
                    return Err(Error::InvalidInput);
                }
            }
            if !loader.is_empty() || !nodes.contains_key(id) {
                nodes.insert(id.into(), (session.clone(), loader, parent, url));
            }
            if let Some(children) = tree["childFrames"].as_array() {
                pending.extend(children.iter().map(|child| (child, Some(id))));
            }
        }
    }
    let first = nodes.remove(root).ok_or(Error::InvalidInput)?;
    let mut result = vec![Plan {
        frame: root.into(),
        session: first.0,
        loader: first.1,
        parent: None,
        url: first.3,
    }];
    let mut depths = vec![0];
    let mut index = 0;
    while index < result.len() {
        if depths[index] < 8 {
            let children: Vec<_> = nodes
                .iter()
                .filter(|(_, entry)| entry.2.as_deref() == Some(&result[index].frame))
                .map(|(id, _)| id.clone())
                .collect();
            for id in children {
                if result.len() >= 64 {
                    return Err(Error::ExecutionFailed);
                }
                let (session, loader, _, url) = nodes.remove(&id).unwrap();
                result.push(Plan {
                    frame: id,
                    loader,
                    session,
                    parent: Some(index),
                    url,
                });
                depths.push(depths[index] + 1);
            }
        }
        index += 1;
    }
    // Disconnected/cyclic/over-depth routes cannot silently count as covered.
    if !nodes.is_empty() {
        return Err(Error::InvalidInput);
    }
    Ok(result)
}

fn allowed(url: &str) -> bool {
    if matches!(url, "about:blank" | "about:srcdoc") {
        return true;
    }
    url::Url::parse(url).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
    })
}

async fn call_world(
    conn: &Connection,
    world: &Document,
    script: &str,
    args: Vec<Value>,
) -> Result<Value, Error> {
    let guarded = format!(
        "function(...args){{if(location.protocol!=='http:'&&location.protocol!=='https:'&&location.href!=='about:blank'&&location.href!=='about:srcdoc')throw new Error('Unsupported document');return ({script}).apply(this,args);}}"
    );
    let mut params = CallFunctionOnParams::new(guarded);
    params.object_id = Some(RemoteObjectId::new(world.object.clone()));
    params.arguments = Some(
        args.into_iter()
            .map(serde_json::from_value)
            .collect::<Result<Vec<CallArgument>, _>>()
            .map_err(|_| Error::InvalidInput)?,
    );
    params.return_by_value = Some(true);
    params.await_promise = Some(true);
    let result = conn
        .send(&world.session, &params)
        .await
        .map_err(|_| Error::InvalidInput)?;
    if result.get("exceptionDetails").is_some() {
        return Err(Error::InvalidInput);
    }
    result["result"]
        .get("value")
        .cloned()
        .ok_or(Error::InvalidInput)
}

impl Documents {
    pub fn activate(&mut self, reference: &str) {
        self.active = self.refs.get(reference).map_or(0, |entry| entry.0);
    }
    pub fn reset_active(&mut self) {
        self.active = 0;
    }
    pub fn has_active_child(&self) -> bool {
        self.active > 0
    }

    pub async fn validate(&mut self, conn: &Connection) -> Result<(), Error> {
        if self.worlds.is_empty() {
            return Ok(());
        }
        let trees = self
            .routes
            .as_mut()
            .ok_or(Error::InvalidInput)?
            .refresh(conn)
            .await?;
        let current = plans(&trees, &self.worlds[0].frame)?;
        for world in &self.worlds {
            let Some(plan) = current.iter().find(|plan| {
                plan.frame == world.frame
                    && plan.session == world.session
                    && plan.loader == world.loader
            }) else {
                return Err(Error::InvalidInput);
            };
            if !allowed(&plan.url) {
                return Err(Error::TabDenied);
            }
            if plan.parent.map(|p| current[p].frame.as_str())
                != world.parent.map(|p| self.worlds[p].frame.as_str())
            {
                return Err(Error::InvalidInput);
            }
        }
        Ok(())
    }

    pub async fn clear(&mut self, conn: &Connection) -> Result<(), Error> {
        self.active = 0;
        self.refs.clear();
        // Release in reverse order. Keep uncertain groups until the caller can
        // prove a retry or detach; releasing this page never closes a target.
        while self.worlds.len() > 1 {
            let world = self.worlds.last().unwrap();
            let parent = world.parent.ok_or(Error::InvalidInput)?;
            let owner_session = &self.worlds[parent].session;
            if conn.registry().has_session(owner_session) {
                conn.send(
                    owner_session,
                    &ReleaseObjectGroupParams::new(world.owner_group.clone()),
                )
                .await
                .map_err(|_| Error::ExecutionFailed)?;
            }
            if conn.registry().has_session(&world.session) {
                conn.send(
                    &world.session,
                    &ReleaseObjectGroupParams::new(world.group.clone()),
                )
                .await
                .map_err(|_| Error::ExecutionFailed)?;
            }
            self.worlds.pop();
        }
        self.worlds.clear();
        Ok(())
    }

    pub async fn observe(
        &mut self,
        conn: &Connection,
        root: Document,
        content: &mut String,
        elements: &mut Vec<Value>,
        cancel: &CancellationToken,
    ) -> Result<usize, Error> {
        self.clear(conn).await?;
        if self.routes.is_none() {
            self.routes = Some(super::frame_routes::FrameRoutes::new(conn, &root.session));
        }
        let trees = self.routes.as_mut().unwrap().refresh(conn).await?;
        let plans = plans(&trees, &root.frame)?;
        if plans[0].loader != root.loader || plans[0].session != root.session {
            return Err(Error::InvalidInput);
        }
        self.worlds.push(root);
        let mut indices = BTreeMap::from([(0, 0)]);
        let mut unobserved = 0;
        for (plan_index, plan) in plans.into_iter().enumerate().skip(1) {
            check(cancel, conn)?;
            let Some(parent) = plan.parent.and_then(|p| indices.get(&p).copied()) else {
                unobserved += 1;
                continue;
            };
            if !allowed(&plan.url) || plan.loader.is_empty() {
                unobserved += 1;
                continue;
            }
            let mut params = CreateIsolatedWorldParams::new(plan.frame.clone());
            params.world_name = Some("nomifun-attached-browser-semantic".into());
            params.grant_univeral_access = Some(false);
            let context = conn
                .send(&plan.session, &params)
                .await
                .map_err(|_| Error::InvalidInput)?["executionContextId"]
                .as_i64()
                .ok_or(Error::InvalidInput)?;
            let index = self.worlds.len();
            self.worlds.push(Document {
                session: plan.session,
                frame: plan.frame,
                loader: plan.loader,
                object: String::new(),
                context,
                group: format!("nomi-attached-frame-{}", nomifun_common::generate_id()),
                parent: Some(parent),
                owner: None,
                owner_group: format!("nomi-attached-frame-owner-{}", nomifun_common::generate_id()),
            });
            let world = &mut self.worlds[index];
            let mut params = EvaluateParams::new(format!(
                "if(location.protocol!=='http:'&&location.protocol!=='https:'&&location.href!=='about:blank'&&location.href!=='about:srcdoc')throw new Error('Unsupported document');\n{}",
                native_semantic::initialization_expression()
            ));
            params.context_id = Some(ExecutionContextId::new(context));
            params.object_group = Some(world.group.clone());
            let result = conn
                .send(&world.session, &params)
                .await
                .map_err(|_| Error::InvalidInput)?;
            if result.get("exceptionDetails").is_some() {
                return Err(Error::InvalidInput);
            }
            world.object = result["result"]["objectId"]
                .as_str()
                .ok_or(Error::InvalidInput)?
                .into();
            let params: GetFrameOwnerParams =
                serde_json::from_value(json!({"frameId":world.frame}))
                    .map_err(|_| Error::InvalidInput)?;
            let node = conn
                .send(&self.worlds[parent].session, &params)
                .await
                .map_err(|_| Error::InvalidInput)?;
            let params:ResolveNodeParams=serde_json::from_value(json!({"backendNodeId":node["backendNodeId"],"executionContextId":self.worlds[parent].context,"objectGroup":self.worlds[index].owner_group})).map_err(|_|Error::InvalidInput)?;
            let owner = conn
                .send(&self.worlds[parent].session, &params)
                .await
                .map_err(|_| Error::InvalidInput)?;
            self.worlds[index].owner = Some(
                owner["object"]["objectId"]
                    .as_str()
                    .ok_or(Error::InvalidInput)?
                    .into(),
            );
            let snapshot =
                call_world(conn, &self.worlds[index], native_semantic::OBSERVE, vec![]).await?;
            let raw = snapshot["elements"]
                .as_array()
                .ok_or(Error::ExecutionFailed)?;
            let mut text = snapshot["content"]
                .as_str()
                .ok_or(Error::ExecutionFailed)?
                .to_owned();
            if elements.len().saturating_add(raw.len()) > 2000
                || content.len().saturating_add(text.len()) > 200_000
            {
                return Err(Error::ExecutionFailed);
            }
            let secrets=call_world(conn,&self.worlds[index],"function(){return Array.from(this._lastAriaSnapshotForQuery?.elements||[]).filter(([,el])=>el.tagName==='INPUT'&&(el.type==='password'||(el.getAttribute('autocomplete')||'').toLowerCase().includes('password'))||el.tagName==='TEXTAREA'&&(el.getAttribute('autocomplete')||'').toLowerCase().includes('password')).map(([ref])=>String(ref));}",vec![]).await?;
            let secrets: Vec<String> =
                serde_json::from_value(secrets).map_err(|_| Error::InvalidInput)?;
            text = crate::redact::redact_yaml(&crate::redact::blank_all_editable_values(
                &crate::redact::blank_secret_values(&text, &secrets),
            ));
            for element in raw {
                let local = element["ref_id"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() < 100)
                    .ok_or(Error::InvalidInput)?;
                let reference = format!("f{index}:{local}");
                text = text.replace(&format!("[ref={local}]"), &format!("[ref={reference}]"));
                self.refs.insert(reference.clone(), (index, local.into()));
                let mut element = element.clone();
                element["ref_id"] = json!(reference);
                element["name"] = json!(crate::redact::redact_yaml(
                    element["name"].as_str().unwrap_or("")
                ));
                elements.push(element);
            }
            content.push_str(&format!(
                "\nFrame f{index}:\n{}",
                crate::redact::wrap_untrusted(&text, None)
            ));
            if content.len() > 200_000 || elements.len() > 2000 {
                return Err(Error::ExecutionFailed);
            }
            indices.insert(plan_index, index);
        }
        self.validate(conn).await?;
        check(cancel, conn)?;
        Ok(unobserved)
    }

    pub async fn call_active(
        &self,
        conn: &Connection,
        script: &str,
        mut args: Vec<Value>,
    ) -> Result<Value, Error> {
        if let Some(reference) = args.first().and_then(Value::as_str) {
            if let Some((index, local)) = self.refs.get(reference) {
                if *index != self.active {
                    return Err(Error::InvalidInput);
                }
                args[0] = json!(local);
            }
        }
        let value = call_world(
            conn,
            &self.worlds[self.active],
            script,
            args.into_iter().map(|v| json!({"value":v})).collect(),
        )
        .await?;
        if script == native_semantic::LOCATE {
            let point = self.map_point(conn, self.active, point(&value)?).await?;
            return Ok(json!({"x":point.0,"y":point.1}));
        }
        if script == native_semantic::IS_FOCUSED && value == true {
            if !self.ancestors_focused(conn).await? {
                return Ok(json!(false));
            }
        }
        Ok(value)
    }

    pub async fn ancestors_focused(&self, conn: &Connection) -> Result<bool, Error> {
        if self.active == 0 {
            return Ok(true);
        }
        let mut index = self.active;
        while let Some(parent) = self.worlds[index].parent {
            let owner = self.worlds[index]
                .owner
                .as_ref()
                .ok_or(Error::InvalidInput)?;
            if call_world(
                conn,
                &self.worlds[parent],
                "function(owner){let active=document.activeElement;while(active?.shadowRoot?.activeElement)active=active.shadowRoot.activeElement;return owner.isConnected&&active===owner;}",
                vec![json!({"objectId":owner})],
            )
            .await?
                != true
            {
                return Ok(false);
            }
            index = parent;
        }
        Ok(true)
    }

    async fn viewport(&self, conn: &Connection, index: usize) -> Result<(f64, f64), Error> {
        let value = call_world(
            conn,
            &self.worlds[index],
            "function(){return {x:innerWidth,y:innerHeight}}",
            vec![],
        )
        .await?;
        point(&value)
    }
    async fn quad(&self, conn: &Connection, index: usize) -> Result<ContentQuad, Error> {
        let world = &self.worlds[index];
        let parent = world.parent.ok_or(Error::InvalidInput)?;
        let params: GetBoxModelParams = serde_json::from_value(json!({"objectId":world.owner}))
            .map_err(|_| Error::InvalidInput)?;
        let value = conn
            .send(&self.worlds[parent].session, &params)
            .await
            .map_err(|_| Error::InvalidInput)?;
        ContentQuad::read(&value["model"]["content"]).map_err(|_| Error::InvalidInput)
    }
    fn session_root(&self, index: usize) -> bool {
        self.worlds[index]
            .parent
            .is_none_or(|parent| self.worlds[index].session != self.worlds[parent].session)
    }
    async fn map_point(
        &self,
        conn: &Connection,
        mut index: usize,
        mut position: (f64, f64),
    ) -> Result<(f64, f64), Error> {
        while let Some(parent) = self.worlds[index].parent {
            let owner = self.worlds[index]
                .owner
                .as_ref()
                .ok_or(Error::InvalidInput)?;
            let ready = call_world(
                conn,
                &self.worlds[parent],
                CHECK_FRAME_OWNER,
                vec![json!({"objectId":owner})],
            )
            .await?;
            if ready["ready"] != true {
                return Err(Error::InvalidInput);
            }
            let viewport = self.viewport(conn, index).await?;
            let quad = self.quad(conn, index).await?;
            let mut mapped = quad
                .project(position, viewport)
                .map_err(|_| Error::InvalidInput)?;
            let parent_quad = if self.session_root(parent) {
                None
            } else {
                Some(self.quad(conn, parent).await?)
            };
            let parent_viewport = self.viewport(conn, parent).await?;
            if let Some(quad) = &parent_quad {
                mapped = quad
                    .unproject(mapped, parent_viewport)
                    .map_err(|_| Error::InvalidInput)?;
            }
            position = point(
                &call_world(
                    conn,
                    &self.worlds[parent],
                    CHECK_FRAME_HIT,
                    vec![
                        json!({"objectId":owner}),
                        json!({"value":mapped.0}),
                        json!({"value":mapped.1}),
                    ],
                )
                .await?,
            )?;
            if !quad.unchanged(&self.quad(conn, index).await?)
                || self.viewport(conn, index).await? != viewport
                || self.viewport(conn, parent).await? != parent_viewport
            {
                return Err(Error::InvalidInput);
            }
            if let Some(quad) = parent_quad {
                if !quad.unchanged(&self.quad(conn, parent).await?) {
                    return Err(Error::InvalidInput);
                }
            }
            index = parent;
        }
        Ok(position)
    }
    pub async fn scroll_point(&self, conn: &Connection) -> Result<(f64, f64), Error> {
        let size = self.viewport(conn, self.active).await?;
        self.map_point(conn, self.active, (size.0 / 2., size.1 / 2.))
            .await
    }
}

fn point(value: &Value) -> Result<(f64, f64), Error> {
    Ok((
        value["x"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or(Error::InvalidInput)?,
        value["y"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or(Error::InvalidInput)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tree(id: &str, parent: Option<&str>, loader: &str) -> Value {
        let mut frame = json!({"id":id,"loaderId":loader,"url":"https://fixture.test/"});
        if let Some(parent) = parent {
            frame["parentId"] = json!(parent);
        }
        json!({"frame":frame})
    }
    #[test]
    fn page_and_oopif_trees_form_one_parent_first_bounded_plan() {
        let mut root = tree("root", None, "r");
        root["childFrames"] = json!([tree("same", Some("root"), "s")]);
        let mut cross = tree("cross", Some("same"), "c");
        cross["childFrames"] = json!([tree("nested", Some("cross"), "n")]);
        let result = plans(&[("page".into(), root), ("iframe".into(), cross)], "root").unwrap();
        assert_eq!(
            result
                .iter()
                .map(|p| (p.frame.as_str(), p.parent, p.session.as_str()))
                .collect::<Vec<_>>(),
            [
                ("root", None, "page"),
                ("same", Some(0), "page"),
                ("cross", Some(1), "iframe"),
                ("nested", Some(2), "iframe")
            ]
        );
    }
    #[test]
    fn foreign_disconnected_or_contradictory_frame_trees_are_not_guessed_into_the_page() {
        for extra in [
            tree("foreign", None, "x"),
            tree("cycle", Some("cycle"), "x"),
            tree("child", Some("missing"), "x"),
        ] {
            assert!(
                plans(
                    &[
                        ("page".into(), tree("root", None, "r")),
                        ("other".into(), extra)
                    ],
                    "root"
                )
                .is_err()
            );
        }
        let mut root = tree("root", None, "r");
        root["childFrames"] = json!([tree("child", Some("root"), "a")]);
        assert!(
            plans(
                &[
                    ("page".into(), root),
                    ("iframe".into(), tree("child", Some("root"), "b"))
                ],
                "root"
            )
            .is_err()
        );
    }
    #[test]
    fn inherited_frame_documents_are_allowed_but_privileged_or_credential_urls_are_not() {
        for url in [
            "about:blank",
            "about:srcdoc",
            "https://fixture.test/form",
            "http://localhost/form",
        ] {
            assert!(allowed(url));
        }
        for url in [
            "chrome://settings",
            "file:///private",
            "data:text/html,secret",
            "https://user:pass@fixture.test/form",
            "",
        ] {
            assert!(!allowed(url));
        }
    }
}
