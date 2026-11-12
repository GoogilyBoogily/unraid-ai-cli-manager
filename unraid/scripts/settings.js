(() => {
  'use strict';
  const root = document.getElementById('ai-cli-manager');
  const message = document.getElementById('manager-message');
  const interval = document.getElementById('backup-interval');
  const csrf = root.dataset.csrf;
  let pending = false;
  async function request(body) {
    const response = await fetch('/plugins/ai-cli-manager/include/actions.php', {
      method: 'POST', credentials: 'same-origin',
      headers: {'Content-Type': 'application/json', 'X-CSRF-Token': csrf},
      body: JSON.stringify({...body, csrf_token: csrf})
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || 'Manager request failed');
    return result;
  }
  function field(list, label, value) {
    const term = document.createElement('dt');
    const description = document.createElement('dd');
    term.textContent = label;
    description.textContent = value;
    list.append(term, description);
  }
  function date(value) { return value ? new Date(value * 1000).toLocaleString() : 'Never'; }
  async function refresh() {
    if (pending) return;
    pending = true;
    try {
      const status = await request({action: 'status', tool: 'all'});
      if (document.activeElement !== interval) interval.value = status.backup_interval_minutes;
      root.querySelectorAll('fieldset[data-tool]').forEach(section => {
        const tool = status.tools[section.dataset.tool];
        const busy = tool.operation === 'busy';
        section.querySelectorAll('[data-setting]').forEach(input => {
          input.checked = tool.settings[input.dataset.setting];
          input.disabled = busy || (input.dataset.setting === 'enabled' && !tool.settings.initialized);
        });
        section.querySelectorAll('[data-action]').forEach(button => {
          button.disabled = busy || (button.dataset.action !== 'initialize' && !tool.settings.initialized);
        });
        const list = section.querySelector('[data-status]');
        list.replaceChildren();
        field(list, 'State', `${tool.readiness} / ${tool.operation}`);
        field(list, 'Installed / cached', `${tool.versions.installed || 'Missing'} / ${tool.versions.cached || 'Missing'}`);
        if (section.dataset.tool === 'codex') field(list, 'Running daemon', tool.versions.daemon.join(', ') || 'Not running');
        field(list, 'Last successful backup', date(tool.last_successful_backup));
        field(list, 'Last successful update', date(tool.last_successful_update));
        field(list, 'Restart required', tool.restart_required ? 'Yes — finish active work, then restart the tool or daemon' : 'No');
        field(list, 'Native update policy', tool.native_update_policy.verified_on_server ? 'Server validated' : 'Unverified — release blocked');
        if (tool.errors.length) field(list, 'Needs attention', tool.errors.map(error => `${error.code}: ${error.error}`).join('\n'));
        if (tool.job) field(list, 'Last job', `${tool.job.job_identifier}: ${tool.job.state}`);
      });
    } catch (error) { message.textContent = error.message; }
    finally { pending = false; }
  }
  root.addEventListener('change', async event => {
    const input = event.target.closest('[data-setting]');
    if (!input) return;
    input.disabled = true;
    try {
      await request({action: 'settings', tool: input.closest('fieldset').dataset.tool, [input.dataset.setting]: input.checked});
      message.textContent = 'Settings saved.';
    } catch (error) { message.textContent = error.message; }
    await refresh();
  });
  root.addEventListener('click', async event => {
    const button = event.target.closest('[data-action], #save-interval');
    if (!button) return;
    button.disabled = true;
    try {
      if (button.id === 'save-interval') {
        await request({action: 'settings', backup_interval_minutes: Number(interval.value)});
        message.textContent = 'Backup interval saved.';
      } else {
        const section = button.closest('fieldset');
        const body = {action: button.dataset.action, tool: section.dataset.tool};
        if (body.action === 'initialize' && body.tool === 'codex') body.host_execution_opt_in = section.querySelector('[data-host-consent]').checked;
        const result = await request(body);
        const identifier = result.job_identifier || result.jobs?.[0]?.receipt?.job_identifier;
        message.textContent = identifier ? `Job started: ${identifier}` : 'Request accepted.';
      }
    } catch (error) { message.textContent = error.message; }
    finally { button.disabled = false; }
    await refresh();
  });
  refresh();
  const timer = setInterval(refresh, 5000);
  window.addEventListener('pagehide', () => clearInterval(timer), {once: true});
})();
