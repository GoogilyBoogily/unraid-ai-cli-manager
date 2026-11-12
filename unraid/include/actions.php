<?php
declare(strict_types=1);
ini_set('display_errors', '0');
header('Content-Type: application/json');
header('Cache-Control: no-store');
require_once __DIR__ . '/request.php';

try {
    $authenticated = false;
    if (isset($_COOKIE[session_name()])) {
        if (!session_start()) {
            throw new RuntimeException('Session unavailable');
        }
        $authenticated = isset($_SESSION['unraid_login']);
        session_write_close();
    }
    if (!$authenticated) {
        http_response_code(403);
        throw new InvalidArgumentException('Authentication required');
    }
    if ((int)($_SERVER['CONTENT_LENGTH'] ?? 0) > 16384) {
        throw new InvalidArgumentException('Request too large');
    }
    $body = file_get_contents('php://input', false, null, 0, 16385);
    if (!is_string($body) || strlen($body) > 16384) {
        throw new InvalidArgumentException('Request too large');
    }
    $request = json_decode($body, true, 32, JSON_THROW_ON_ERROR);
    if (!is_array($request)) {
        throw new InvalidArgumentException('Expected a request object');
    }
    $state = parse_ini_file('/var/local/emhttp/var.ini', false, INI_SCANNER_RAW);
    $command = ai_manager_request($request, $authenticated, $_SERVER['REQUEST_METHOD'] ?? '', is_array($state) ? ($state['csrf_token'] ?? '') : '');
    $output = tmpfile();
    $diagnostics = tmpfile();
    if ($output === false || $diagnostics === false) {
        throw new RuntimeException('Private output unavailable');
    }
    $process = proc_open(array_merge(['/usr/local/sbin/ai-cli-manager'], $command['arguments']),
        [0 => ['pipe', 'r'], 1 => $output, 2 => $diagnostics], $pipes, '/', null, ['bypass_shell' => true]);
    if (!is_resource($process)) {
        throw new RuntimeException('Backend unavailable');
    }
    fwrite($pipes[0], $command['stdin']);
    fclose($pipes[0]);
    $deadline = microtime(true) + 10;
    do {
        $status = proc_get_status($process);
        if (!$status['running']) {
            break;
        }
        if (microtime(true) >= $deadline) {
            proc_terminate($process, 9);
            proc_close($process);
            throw new RuntimeException('Backend response timed out');
        }
        usleep(10000);
    } while (true);
    proc_close($process);
    rewind($output);
    rewind($diagnostics);
    $response = stream_get_contents($status['exitcode'] === 0 ? $output : $diagnostics, 1048576);
    $value = json_decode($response, true, 32, JSON_THROW_ON_ERROR);
    if (!is_array($value)) {
        throw new RuntimeException('Invalid backend response');
    }
    if ($status['exitcode'] !== 0) {
        http_response_code(409);
    }
    echo json_encode($value, JSON_THROW_ON_ERROR);
} catch (InvalidArgumentException | JsonException $error) {
    if (http_response_code() < 400) http_response_code(400);
    echo json_encode(['error' => 'Request rejected']);
} catch (Throwable $error) {
    http_response_code(503);
    error_log('ai-cli-manager: request backend failed (' . get_class($error) . ')');
    echo json_encode(['error' => 'Manager unavailable. Check system logs.']);
}
