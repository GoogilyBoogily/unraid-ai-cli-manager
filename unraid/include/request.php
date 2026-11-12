<?php
declare(strict_types=1);

function ai_manager_request(array $request, bool $authenticated, string $method, string $expectedToken): array
{
    if (!$authenticated || $method !== 'POST' || $expectedToken === '' ||
        !is_string($request['csrf_token'] ?? null) || !hash_equals($expectedToken, $request['csrf_token'])) {
        throw new InvalidArgumentException('Authentication or CSRF validation failed');
    }
    $action = $request['action'] ?? null;
    $fields = [
        'status' => ['tool'], 'job' => ['identifier'],
        'initialize' => ['tool', 'host_execution_opt_in'],
        'backup' => ['tool'], 'restore' => ['tool'], 'update' => ['tool'],
        'settings' => ['tool', 'enabled', 'automatic_updates', 'backup_interval_minutes'],
    ];
    if (!is_string($action) || !isset($fields[$action]) ||
        array_diff(array_keys($request), array_merge(['action', 'csrf_token'], $fields[$action]))) {
        throw new InvalidArgumentException('Unsupported request');
    }
    $tool = $request['tool'] ?? ($action === 'status' ? 'all' : null);
    if ($tool !== null && !in_array($tool, $action === 'status' ? ['claude', 'codex', 'all'] : ['claude', 'codex'], true)) {
        throw new InvalidArgumentException('Invalid tool');
    }
    foreach (['enabled', 'automatic_updates', 'host_execution_opt_in'] as $field) {
        if (array_key_exists($field, $request) && !is_bool($request[$field])) {
            throw new InvalidArgumentException('Expected a boolean');
        }
    }
    if ($action === 'job') {
        $identifier = $request['identifier'] ?? null;
        if (!is_string($identifier) || !preg_match('/\A[a-zA-Z0-9-]{1,128}\z/', $identifier)) {
            throw new InvalidArgumentException('Invalid job identifier');
        }
        return ['arguments' => ['__internal', 'job', '--identifier', $identifier], 'stdin' => ''];
    }
    if ($action === 'settings') {
        $settings = array_intersect_key($request, array_flip($fields[$action]));
        if (isset($settings['backup_interval_minutes']) && !in_array($settings['backup_interval_minutes'], [5, 15, 30, 60], true)) {
            throw new InvalidArgumentException('Invalid backup interval');
        }
        if ($tool === null && (isset($settings['enabled']) || isset($settings['automatic_updates']))) {
            throw new InvalidArgumentException('Select a tool');
        }
        return ['arguments' => ['__internal', 'settings'], 'stdin' => json_encode($settings, JSON_THROW_ON_ERROR)];
    }
    if (!is_string($tool)) {
        throw new InvalidArgumentException('Select a tool');
    }
    if ($action === 'initialize') {
        $arguments = ['__internal', 'initialize', '--tool', $tool];
        if (($request['host_execution_opt_in'] ?? false) === true) {
            if ($tool !== 'codex') {
                throw new InvalidArgumentException('Host execution consent applies to Codex');
            }
            $arguments[] = '--host-execution-opt-in';
        }
    } else {
        $arguments = [$action, '--tool', $tool, '--json'];
    }
    return ['arguments' => $arguments, 'stdin' => ''];
}
