"""Pure fixtures only; no sockets, native workloads or /proc probes."""
import unittest
from tcp_info import parse_ss, deltas


def line(inode=1, pid=42, **overrides):
    fields = dict(segs_out=200, segs_in=150, data_segs_out=100,
                  data_segs_in=100, bytes_sent=700)
    fields.update(overrides)
    return (f'0 0 127.0.0.1:1234 127.0.0.1:4321 users:(("app",pid={pid},fd=5)) '
            f'ino:{inode} sk:123 ' + ' '.join(f'{k}:{v}' for k, v in fields.items()))


class TcpInfoTests(unittest.TestCase):
    def test_counts_ack_only_segments_and_keeps_identity(self):
        before = parse_ss(line(), 42, expected=1)
        after = parse_ss(line(segs_out=500, data_segs_out=300, bytes_sent=2100), 42, expected=1)
        result = deltas(before, after)
        self.assertEqual(result['ack_only_out'], 100)
        self.assertEqual(result['data_segs_out'], 200)
        self.assertEqual(result['ack_only_per_data_out'], .5)

    def test_all_eight_owned_sockets_required(self):
        self.assertEqual(len(parse_ss('\n'.join(line(inode=i) for i in range(8)), 42)), 8)
        with self.assertRaises(ValueError):
            parse_ss(line(), 42)
        with self.assertRaises(ValueError):
            parse_ss('', 42)

    def test_other_pid_duplicate_inode_and_missing_counter_fail(self):
        for raw in [line(pid=999), line() + '\n' + line(),
                    line().replace('data_segs_out:100', '')]:
            with self.assertRaises(ValueError):
                parse_ss(raw, 42, expected=1)

    def test_counter_regression_and_socket_replacement_fail(self):
        before = parse_ss(line(), 42, expected=1)
        for raw in [line(segs_out=199), line(inode=2), line(segs_out=200, data_segs_out=100)]:
            with self.assertRaises(ValueError):
                deltas(before, parse_ss(raw, 42, expected=1))


if __name__ == '__main__':
    unittest.main()
