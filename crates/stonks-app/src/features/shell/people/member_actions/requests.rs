use super::*;
impl MemberActions {
    /// Ticket verification and ownership mutation have separate owners: the
    /// completed MFA task must not be dropped while its callback is executing.
    pub(super) fn begin_transfer(
        &mut self,
        target: Target,
        ticket: String,
        cx: &mut Context<Self>,
    ) {
        let Some(generation) = self.gate.start() else {
            return;
        };
        let policy = self.policy.clone();
        let authorization = self.session.read(cx).authorization();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let result = spawn_core(Abortable::new(
                async move {
                    service::transfer_with_ticket(
                        &policy,
                        &authorization,
                        &target.server,
                        &target.user,
                        &ticket,
                    )
                    .await
                },
                registration,
            ))
            .await;
            this.update(cx, |this, cx| {
                if !this.gate.complete(generation) {
                    return;
                }
                this.profile_pending = None;
                match result {
                    Ok(Ok(server)) => {
                        cx.emit(MemberActionsEvent::UpdatedServer(server));
                        this.finish_close(cx);
                    }
                    Ok(Err(error)) => this.error = Some(error.to_string()),
                    Err(_) => this.error = Some("The ownership update was cancelled.".into()),
                }
                cx.notify();
            })
            .ok();
        });
        self.transfer_request = Some(Request { abort, _task: task });
        cx.notify();
    }
    /// Window-aware completion lets the MFA input retain a rejected value and
    /// clear it immediately when the operation succeeds.
    pub(super) fn run_in<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        future: impl Future<Output = Result<T, CommandError>> + Send + 'static,
        done: impl FnOnce(&mut Self, Result<T, String>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(ticket) = self.gate.start() else {
            return;
        };
        self.error = None;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = spawn_core(Abortable::new(future, registration)).await;
            this.update_in(cx, |this, window, cx| {
                if !this.gate.complete(ticket) {
                    return;
                }
                this.profile_pending = None;
                match result {
                    Ok(result) => done(this, result.map_err(|error| error.to_string()), window, cx),
                    Err(_) => this.error = Some("The member request was cancelled.".into()),
                }
                cx.notify();
            })
            .ok();
        });
        self.request = Some(Request { abort, _task: task });
        cx.notify();
    }
}
