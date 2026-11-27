// Pinned go-redis client matrix harness for Rillcache readiness C7.
// Every declared case is reported exactly once; a failing case never aborts others.
package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"runtime/debug"
	"strings"
	"time"

	"github.com/redis/go-redis/v9"
)

type caseResult struct {
	ID     string `json:"id"`
	Status string `json:"status"`
	Detail string `json:"detail,omitempty"`
}

type output struct {
	Client    string       `json:"client"`
	Version   string       `json:"version"`
	Protocols []int        `json:"protocols"`
	Cases     []caseResult `json:"cases"`
}

type options struct {
	plainPort       int
	tlsPort         int
	altTLSPort      int
	namedUser       string
	namedPassword   string
	limitedUser     string
	limitedPassword string
	defaultPassword string
	ca              string
	jsonOut         string
}

func main() {
	var opts options
	flag.IntVar(&opts.plainPort, "plain-port", 0, "plaintext candidate port")
	flag.IntVar(&opts.tlsPort, "tls-port", 0, "TLS candidate port")
	flag.IntVar(&opts.altTLSPort, "alt-tls-port", 0, "wrong-identity TLS candidate port")
	flag.StringVar(&opts.namedUser, "named-user", "", "named ACL user")
	flag.StringVar(&opts.namedPassword, "named-password", "", "named ACL password")
	flag.StringVar(&opts.limitedUser, "limited-user", "", "limited ACL user")
	flag.StringVar(&opts.limitedPassword, "limited-password", "", "limited ACL password")
	flag.StringVar(&opts.defaultPassword, "default-password", "", "default user password")
	flag.StringVar(&opts.ca, "ca", "", "test CA certificate")
	flag.StringVar(&opts.jsonOut, "json-out", "", "case results path")
	flag.Parse()
	if opts.plainPort == 0 || opts.tlsPort == 0 || opts.altTLSPort == 0 || opts.namedUser == "" ||
		opts.namedPassword == "" || opts.limitedUser == "" || opts.limitedPassword == "" ||
		opts.defaultPassword == "" || opts.ca == "" || opts.jsonOut == "" {
		fmt.Fprintln(os.Stderr, "all flags are required")
		os.Exit(2)
	}

	version := moduleVersion()
	results := []caseResult{}
	run := func(id string, body func() error) {
		if err := body(); err != nil {
			results = append(results, caseResult{ID: id, Status: "failed", Detail: truncate(err.Error())})
			return
		}
		results = append(results, caseResult{ID: id, Status: "passed"})
	}

	ctx := context.Background()
	plain := func(protocol int) *redis.Client {
		return redis.NewClient(&redis.Options{
			Addr:     fmt.Sprintf("127.0.0.1:%d", opts.plainPort),
			Username: opts.namedUser, Password: opts.namedPassword, DB: 0, Protocol: protocol,
			DialTimeout: 3 * time.Second, ReadTimeout: 3 * time.Second, WriteTimeout: 3 * time.Second,
			MaxRetries: 0,
		})
	}
	key := func(label string) string { return "go-matrix:" + label }

	run("handshake-auth", func() error {
		client := plain(2)
		defer client.Close()
		if got, err := client.Ping(ctx).Result(); err != nil || got != "PONG" {
			return fmt.Errorf("PING returned %q: %v", got, err)
		}
		return nil
	})

	run("acl-allowed", func() error {
		client := plain(2)
		defer client.Close()
		if err := client.Set(ctx, key("acl"), "value", 0).Err(); err != nil {
			return err
		}
		got, err := client.Get(ctx, key("acl")).Result()
		if err != nil || got != "value" {
			return fmt.Errorf("GET returned %q: %v", got, err)
		}
		return nil
	})

	run("acl-denied-command", func() error {
		client := redis.NewClient(&redis.Options{
			Addr: fmt.Sprintf("127.0.0.1:%d", opts.plainPort), Username: opts.limitedUser,
			Password: opts.limitedPassword, DB: 0, Protocol: 2, MaxRetries: 0,
		})
		defer client.Close()
		err := client.Set(ctx, key("denied"), "value", 0).Err()
		if err == nil {
			return errors.New("limited user was allowed to SET")
		}
		if !strings.Contains(strings.ToUpper(err.Error()), "NOPERM") {
			return fmt.Errorf("unexpected error: %v", err)
		}
		return nil
	})

	for _, protocol := range []int{2, 3} {
		protocol := protocol
		run(fmt.Sprintf("resp%d-roundtrip", protocol), func() error {
			client := plain(protocol)
			defer client.Close()
			if got, err := client.Ping(ctx).Result(); err != nil || got != "PONG" {
				return fmt.Errorf("RESP%d PING returned %q: %v", protocol, got, err)
			}
			binary := []byte{0x00, 0xff, 0x0d, 0x0a}
			if err := client.Set(ctx, key(fmt.Sprintf("binary-%d", protocol)), binary, 0).Err(); err != nil {
				return err
			}
			read, err := client.Get(ctx, key(fmt.Sprintf("binary-%d", protocol))).Bytes()
			if err != nil || string(read) != string(binary) {
				return fmt.Errorf("RESP%d binary round-trip differed: %v", protocol, err)
			}
			if err := client.Set(ctx, key(fmt.Sprintf("counter-%d", protocol)), "41", 0).Err(); err != nil {
				return err
			}
			if value, err := client.Incr(ctx, key(fmt.Sprintf("counter-%d", protocol))).Result(); err != nil || value != 42 {
				return fmt.Errorf("RESP%d INCR returned %d: %v", protocol, value, err)
			}
			if err := client.HSet(ctx, key(fmt.Sprintf("hash-%d", protocol)), "field", "value").Err(); err != nil {
				return err
			}
			fields, err := client.HGetAll(ctx, key(fmt.Sprintf("hash-%d", protocol))).Result()
			if err != nil || fields["field"] != "value" {
				return fmt.Errorf("RESP%d HGETALL differed: %v", protocol, err)
			}
			return nil
		})
	}

	run("pipeline", func() error {
		client := plain(2)
		defer client.Close()
		pipe := client.Pipeline()
		pipe.Set(ctx, key("p1"), "one", 0)
		pipe.Set(ctx, key("p2"), "two", 0)
		values := pipe.MGet(ctx, key("p1"), key("p2"))
		if _, err := pipe.Exec(ctx); err != nil {
			return err
		}
		got := values.Val()
		if len(got) != 2 || got[0] != "one" || got[1] != "two" {
			return fmt.Errorf("pipeline replies differed: %v", got)
		}
		return nil
	})

	run("transaction", func() error {
		client := plain(2)
		defer client.Close()
		var read *redis.StringCmd
		if _, err := client.TxPipelined(ctx, func(pipe redis.Pipeliner) error {
			pipe.Set(ctx, key("tx"), "value", 0)
			read = pipe.Get(ctx, key("tx"))
			return nil
		}); err != nil {
			return err
		}
		if got, err := read.Result(); err != nil || got != "value" {
			return fmt.Errorf("transaction result %q: %v", got, err)
		}
		if got, err := client.Get(ctx, key("tx")).Result(); err != nil || got != "value" {
			return fmt.Errorf("transaction state %q: %v", got, err)
		}
		return nil
	})

	run("pubsub", func() error {
		client := plain(2)
		defer client.Close()
		subscriber := client.Subscribe(ctx, key("channel"))
		defer subscriber.Close()
		if _, err := subscriber.Receive(ctx); err != nil {
			return err
		}
		if count, err := client.Publish(ctx, key("channel"), "payload").Result(); err != nil || count != 1 {
			return fmt.Errorf("publish subscriber count %d: %v", count, err)
		}
		message, err := subscriber.ReceiveMessage(ctx)
		if err != nil || message.Payload != "payload" {
			return fmt.Errorf("pub/sub payload differed: %v", err)
		}
		return nil
	})

	run("error-wrong-type", func() error {
		client := plain(2)
		defer client.Close()
		if err := client.Set(ctx, key("wrong"), "value", 0).Err(); err != nil {
			return err
		}
		err := client.LPush(ctx, key("wrong"), "x").Err()
		if err == nil {
			return errors.New("wrong-type write succeeded")
		}
		if !strings.Contains(err.Error(), "WRONGTYPE") {
			return fmt.Errorf("unexpected error: %v", err)
		}
		return nil
	})

	run("error-unknown-command", func() error {
		client := plain(2)
		defer client.Close()
		err := client.Do(ctx, "NOSUCHCOMMAND").Err()
		if err == nil {
			return errors.New("unknown command succeeded")
		}
		if !strings.Contains(strings.ToLower(err.Error()), "unknown command") {
			return fmt.Errorf("unexpected error: %v", err)
		}
		if got, err := client.Ping(ctx).Result(); err != nil || got != "PONG" {
			return fmt.Errorf("connection unusable after the error: %v", err)
		}
		return nil
	})

	run("error-auth-required", func() error {
		client := redis.NewClient(&redis.Options{
			Addr: fmt.Sprintf("127.0.0.1:%d", opts.plainPort), DB: 0, Protocol: 2, MaxRetries: 0,
		})
		defer client.Close()
		err := client.Get(ctx, key("noauth")).Err()
		if err == nil {
			return errors.New("unauthenticated read succeeded")
		}
		upper := strings.ToUpper(err.Error())
		if !strings.Contains(upper, "NOAUTH") && !strings.Contains(upper, "AUTHENTICATION") {
			return fmt.Errorf("unexpected error: %v", err)
		}
		return nil
	})

	run("reconnect", func() error {
		first := plain(2)
		if err := first.Set(ctx, key("reconnect"), "value", 0).Err(); err != nil {
			first.Close()
			return err
		}
		first.Close()
		second := plain(2)
		defer second.Close()
		if got, err := second.Get(ctx, key("reconnect")).Result(); err != nil || got != "value" {
			return fmt.Errorf("value after reconnect %q: %v", got, err)
		}
		return nil
	})

	run("tls-positive", func() error {
		client := tlsClient(opts.tlsPort, opts, opts.namedUser, opts.namedPassword)
		defer client.Close()
		if got, err := client.Ping(ctx).Result(); err != nil || got != "PONG" {
			return fmt.Errorf("TLS PING returned %q: %v", got, err)
		}
		if err := client.Set(ctx, key("tls"), "value", 0).Err(); err != nil {
			return err
		}
		if got, err := client.Get(ctx, key("tls")).Result(); err != nil || got != "value" {
			return fmt.Errorf("TLS GET returned %q: %v", got, err)
		}
		return nil
	})

	run("tls-wrong-identity", func() error {
		client := tlsClient(opts.altTLSPort, opts, opts.namedUser, opts.namedPassword)
		defer client.Close()
		err := client.Ping(ctx).Err()
		if err == nil {
			return errors.New("certificate without the target IP SAN was accepted")
		}
		lower := strings.ToLower(err.Error())
		if !strings.Contains(lower, "certificate") && !strings.Contains(lower, "x509") &&
			!strings.Contains(lower, "hostname") {
			return fmt.Errorf("unexpected TLS error: %v", err)
		}
		return nil
	})

	payload := output{Client: "go-redis", Version: version, Protocols: []int{2, 3}, Cases: results}
	encoded, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.WriteFile(opts.jsonOut, append(encoded, '\n'), 0o644); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	failed := 0
	for _, result := range results {
		if result.Status != "passed" {
			failed++
			fmt.Fprintf(os.Stderr, "FAILED %s: %s\n", result.ID, result.Detail)
		}
	}
	fmt.Printf("go-redis %s: %d/%d cases passed\n", version, len(results)-failed, len(results))
	if failed > 0 {
		os.Exit(1)
	}
}

func tlsClient(port int, opts options, user, password string) *redis.Client {
	pool := x509.NewCertPool()
	pem, err := os.ReadFile(opts.ca)
	if err == nil {
		pool.AppendCertsFromPEM(pem)
	}
	return redis.NewClient(&redis.Options{
		Addr: fmt.Sprintf("127.0.0.1:%d", port), Username: user, Password: password, DB: 0, Protocol: 2,
		TLSConfig: &tls.Config{RootCAs: pool, MinVersion: tls.VersionTLS12}, MaxRetries: 0,
		DialTimeout: 3 * time.Second, ReadTimeout: 3 * time.Second, WriteTimeout: 3 * time.Second,
	})
}

func moduleVersion() string {
	info, ok := debug.ReadBuildInfo()
	if !ok {
		return "unknown"
	}
	for _, dependency := range info.Deps {
		if strings.HasPrefix(dependency.Path, "github.com/redis/go-redis/v9") {
			return dependency.Version
		}
	}
	return "unknown"
}

func truncate(text string) string {
	if len(text) > 400 {
		return text[:400]
	}
	return text
}
