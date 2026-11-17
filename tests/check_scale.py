"""Run a generated fictional CSV case and compare full-count controls.

Usage: python tests/check_scale.py <workshop-cli.exe> <row-count>
Generate the input first with tests/generate_scale.py. This does not set a
performance target; it records observed elapsed time, peak process working set
and output disk size on Windows.
"""
from __future__ import annotations

import json
import ctypes
import subprocess
import sys
import tempfile
import time
from decimal import Decimal
from pathlib import Path
from ctypes import wintypes

PROJECT = Path(__file__).resolve().parents[1]
FLOW = PROJECT / "tests" / "golden"
DATA = PROJECT / "data" / "synthetic" / "scale"
OUTPUT = PROJECT / "outputs" / "scale-runs"


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]


def run_with_memory(binary: Path, *args: str) -> tuple[dict, int]:
    if sys.platform != "win32":
        raise RuntimeError("当前峰值内存采集仅支持 Windows")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessMemoryCounters), wintypes.DWORD]
    psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
    with tempfile.TemporaryFile(mode="w+t", encoding="utf-8") as stdout_file, tempfile.TemporaryFile(
        mode="w+t", encoding="utf-8"
    ) as stderr_file:
        with subprocess.Popen([str(binary), *args], stdout=stdout_file, stderr=stderr_file) as process:
            handle = kernel.OpenProcess(0x0400 | 0x0010, False, process.pid)
            if not handle:
                process.kill()
                raise ctypes.WinError(ctypes.get_last_error())
            peak = 0
            try:
                while process.poll() is None:
                    counters = ProcessMemoryCounters()
                    counters.cb = ctypes.sizeof(counters)
                    if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                        peak = max(peak, counters.PeakWorkingSetSize)
                    time.sleep(0.1)
            finally:
                kernel.CloseHandle(handle)
            stdout_file.seek(0)
            stderr_file.seek(0)
            stdout, stderr = stdout_file.read(), stderr_file.read()
            if process.returncode:
                raise subprocess.CalledProcessError(process.returncode, process.args, stdout, stderr)
            if not peak:
                raise RuntimeError("运行过快或系统拒绝读取内存；峰值内存未采集")
            return json.loads(stdout), peak


def call(binary: Path, *args: str) -> dict:
    completed = subprocess.run(
        [str(binary), *args], capture_output=True, text=True, encoding="utf-8", check=True
    )
    return json.loads(completed.stdout)


def check(binary: Path, rows: int) -> dict:
    case = DATA / str(rows)
    expected = json.loads((case / "expected.json").read_text(encoding="utf-8"))
    config = case / "company.json"
    if expected["rows_per_file"] != rows:
        raise AssertionError("预期值与输入行数不一致")
    started = time.perf_counter()
    run, peak_memory = run_with_memory(binary, "run", str(FLOW), str(config), str(OUTPUT))
    elapsed = time.perf_counter() - started
    reviews = {item["node_id"]: item for item in run["reviews"]}
    assert reviews["je_source"]["rows"] == rows
    assert reviews["tb_source"]["rows"] == rows
    assert reviews["je_decimal"]["exceptions"] == 0
    assert reviews["tb_decimal"]["exceptions"] == 0
    assert reviews["je_filter"]["excluded"] == 0
    assert reviews["compare"]["rows"] == len(expected["groups"])
    assert reviews["compare"]["matched"] == len(expected["groups"])
    assert reviews["compare"]["left_only"] == 0
    assert reviews["compare"]["right_only"] == 0
    assert reviews["compare"]["unresolved"] == 0
    preview = call(binary, "preview", run["database"], "n_compare", "50")
    actual = [dict(zip(preview["columns"], row)) for row in preview["rows"]]
    assert preview["total_rows"] == len(actual) == len(expected["groups"])
    for item in actual:
        amount = Decimal(expected["groups"][item["account_code"]])
        assert Decimal(item["left_amount"]) == amount
        assert Decimal(item["right_amount"]) == amount
        assert Decimal(item["difference"]) == Decimal(0)
        assert item["match_status"] == "数值相符"
    assert sum((Decimal(item["left_amount"]) for item in actual), Decimal()) == Decimal(expected["je_net"])
    assert sum((Decimal(item["right_amount"]) for item in actual), Decimal()) == Decimal(expected["tb_net"])
    run_dir = Path(run["run_dir"])
    metrics = {
        "rows_per_file": rows,
        "elapsed_seconds": round(elapsed, 3),
        "run_disk_bytes": sum(file.stat().st_size for file in run_dir.rglob("*") if file.is_file()),
        "memory_peak_bytes": peak_memory,
        "memory_note": "Windows 进程峰值工作集；仅含本次 CLI 进程，不含系统文件缓存。",
        "checks": "行数、异常、未纳入、完整关联、逐组金额和总额均通过",
    }
    (run_dir / "scale_metrics.json").write_text(json.dumps(metrics, ensure_ascii=False, indent=2), encoding="utf-8")
    return metrics


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    print(json.dumps(check(Path(sys.argv[1]), int(sys.argv[2])), ensure_ascii=False, indent=2))
