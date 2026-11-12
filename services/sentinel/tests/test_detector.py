from models import SentinelConfig
from monitor import NS, AnomalyDetector


def cfg(**kw):
    base = {"velocity_threshold": 4, "velocity_window_secs": 60, "failure_threshold": 3, "failure_window_secs": 100}
    return SentinelConfig(**{**base, **kw})


def test_velocity_triggers_when_count_exceeds_threshold():
    d = AnomalyDetector(cfg())
    out = []
    for i in range(5):
        out += d.on_transfer_requested("a", True, i * NS)
    # threshold 4 => the 5th request in the window exceeds it
    assert [x.pattern for x in out] == ["velocity_exceeded"]
    assert out[0].observed == 5 and out[0].threshold == 4


def test_exactly_threshold_does_not_trigger():
    d = AnomalyDetector(cfg())
    out = [x for i in range(4) for x in d.on_transfer_requested("a", True, i * NS)]
    assert out == []


def test_window_expiry_resets_the_counter():
    d = AnomalyDetector(cfg())
    for i in range(4):
        d.on_transfer_requested("a", True, i * NS)
    out = d.on_transfer_requested("a", True, 100 * NS)  # all earlier ones are > 60s old
    assert out == [] and d.request_count("a") == 1


def test_velocity_counts_rejected_requests_and_is_per_agent():
    d = AnomalyDetector(cfg())
    for i in range(4):
        d.on_transfer_requested("b", False, i * NS)
    assert d.on_transfer_requested("a", True, 5 * NS) == []
    assert len(d.on_transfer_requested("b", False, 5 * NS)) >= 1


def test_velocity_does_not_refire_until_window_passes():
    d = AnomalyDetector(cfg())
    fired = [x for i in range(12) for x in d.on_transfer_requested("a", True, i * NS) if x.pattern == "velocity_exceeded"]
    assert len(fired) == 1
    again = d.on_transfer_requested("a", True, 80 * NS)  # a full window later, still high? count resets
    assert again == []


def test_consecutive_failures_trigger_and_success_resets():
    d = AnomalyDetector(cfg(velocity_threshold=10000))
    assert d.on_transfer_requested("a", False, 1 * NS) == []
    assert d.on_transfer_requested("a", False, 2 * NS) == []
    assert [x.pattern for x in d.on_transfer_requested("a", False, 3 * NS)] == ["consecutive_failures"]
    # one alert per streak
    assert d.on_transfer_requested("a", False, 4 * NS) == []
    # success resets the streak and re-arms
    d.on_transfer_requested("a", True, 5 * NS)
    assert d.failure_count("a") == 0
    for t in (6, 7):
        assert d.on_transfer_requested("a", False, t * NS) == []
    assert len(d.on_transfer_requested("a", False, 8 * NS)) == 1


def test_failures_outside_window_are_dropped():
    d = AnomalyDetector(cfg(velocity_threshold=10000))
    d.on_transfer_requested("a", False, 1 * NS)
    d.on_transfer_requested("a", False, 2 * NS)
    assert d.on_transfer_requested("a", False, 500 * NS) == []  # first two expired
    assert d.failure_count("a") == 1


def test_unpause_event_resets_state():
    d = AnomalyDetector(cfg())
    for i in range(5):
        d.on_transfer_requested("a", True, i * NS)
    d.on_agent_unpaused("a")
    assert d.request_count("a") == 0
    out = [x for i in range(5) for x in d.on_transfer_requested("a", True, (10 + i) * NS)]
    assert len(out) == 1  # can trigger again after being re-activated


def test_config_bounds():
    import pytest
    from pydantic import ValidationError

    for bad in ({"velocity_threshold": 0}, {"velocity_threshold": 10001}, {"velocity_window_secs": 9},
                {"velocity_window_secs": 3601}, {"failure_threshold": 0}, {"failure_threshold": 101}):
        with pytest.raises(ValidationError):
            SentinelConfig(**bad)
    d = SentinelConfig()
    assert (d.velocity_threshold, d.velocity_window_secs, d.failure_threshold, d.failure_window_secs) == (20, 60, 5, 300)


def test_config_from_env():
    c = SentinelConfig.from_env({"VELOCITY_THRESHOLD": "4", "NEAR_RPC_URL": "http://x:1", "CORS_ORIGINS": "http://a, http://b"})
    assert c.velocity_threshold == 4 and c.rpc_url == "http://x:1" and c.cors_origins == ["http://a", "http://b"]
