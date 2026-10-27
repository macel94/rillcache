package main

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"io"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"
)

type identity struct {
	certPEM []byte
	keyPEM  []byte
	pool    *x509.CertPool
}

func newIdentity(t *testing.T, parent *identity, cn string, server, client bool) identity {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 120))
	if err != nil {
		t.Fatal(err)
	}
	tmpl := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: cn},
		NotBefore:             time.Now().Add(-time.Hour),
		NotAfter:              time.Now().Add(time.Hour),
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		BasicConstraintsValid: true,
		IsCA:                  parent == nil,
		DNSNames:              []string{"localhost"},
		IPAddresses:           []net.IP{net.ParseIP("127.0.0.1")},
	}
	if server {
		tmpl.ExtKeyUsage = append(tmpl.ExtKeyUsage, x509.ExtKeyUsageServerAuth)
	}
	if client {
		tmpl.ExtKeyUsage = append(tmpl.ExtKeyUsage, x509.ExtKeyUsageClientAuth)
	}
	parentCert := tmpl
	parentKey := key
	pool := x509.NewCertPool()
	if parent != nil {
		block, _ := pem.Decode(parent.certPEM)
		if block == nil {
			t.Fatal("parent certificate is not PEM")
		}
		parentCert, err = x509.ParseCertificate(block.Bytes)
		if err != nil {
			t.Fatal(err)
		}
		keyBlock, _ := pem.Decode(parent.keyPEM)
		if keyBlock == nil {
			t.Fatal("parent key is not PEM")
		}
		parentKey, err = x509.ParseECPrivateKey(keyBlock.Bytes)
		if err != nil {
			t.Fatal(err)
		}
		pool = parent.pool
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, parentCert, &key.PublicKey, parentKey)
	if err != nil {
		t.Fatal(err)
	}
	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	keyDER, err := x509.MarshalECPrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: keyDER})
	leaf, err := x509.ParseCertificate(der)
	if err != nil {
		t.Fatal(err)
	}
	if parent == nil {
		pool = x509.NewCertPool()
	}
	pool.AddCert(leaf)
	return identity{certPEM: certPEM, keyPEM: keyPEM, pool: pool}
}

func writeIdentity(t *testing.T, id identity) (certPath, keyPath string) {
	t.Helper()
	dir := t.TempDir()
	certPath = filepath.Join(dir, "cert.pem")
	keyPath = filepath.Join(dir, "key.pem")
	if err := os.WriteFile(certPath, id.certPEM, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(keyPath, id.keyPEM, 0o600); err != nil {
		t.Fatal(err)
	}
	return certPath, keyPath
}

func startBackend(t *testing.T) string {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = listener.Close() })
	go func() {
		for {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			go func() {
				defer conn.Close()
				_, _ = io.Copy(conn, conn)
			}()
		}
	}()
	return listener.Addr().String()
}

func startProxy(t *testing.T, config Config) *Server {
	t.Helper()
	server, err := New(config)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	if err := server.Start(ctx); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		shutdown, stop := context.WithTimeout(context.Background(), 2*time.Second)
		defer stop()
		_ = server.Shutdown(shutdown)
	})
	return server
}

func echoOnce(t *testing.T, addr string, config *tls.Config) (tls.ConnectionState, []byte) {
	t.Helper()
	conn, err := tls.Dial("tcp", addr, config)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	payload := []byte("PING\r\n")
	if _, err := conn.Write(payload); err != nil {
		t.Fatal(err)
	}
	reply := make([]byte, len(payload))
	if _, err := io.ReadFull(conn, reply); err != nil {
		t.Fatal(err)
	}
	return conn.ConnectionState(), reply
}

func TestTLS13ProxyEchoesRESPAndRejectsOldTLS(t *testing.T) {
	serverID := newIdentity(t, nil, "localhost", true, false)
	certPath, keyPath := writeIdentity(t, serverID)
	proxy := startProxy(t, Config{
		ListenAddr:       "127.0.0.1:0",
		BackendAddr:      startBackend(t),
		CertFile:         certPath,
		KeyFile:          keyPath,
		MaxConns:         8,
		HandshakeTimeout: 2 * time.Second,
		DialTimeout:      2 * time.Second,
	})
	config := &tls.Config{
		MinVersion: tls.VersionTLS13,
		MaxVersion: tls.VersionTLS13,
		RootCAs:    serverID.pool,
		ServerName: "localhost",
		NextProtos: []string{"h2"},
	}
	state, reply := echoOnce(t, proxy.Addr().String(), config)
	if string(reply) != "PING\r\n" {
		t.Fatalf("unexpected reply %q", reply)
	}
	if state.Version != tls.VersionTLS13 {
		t.Fatalf("negotiated version %x", state.Version)
	}
	if state.NegotiatedProtocol != "" {
		t.Fatalf("unexpected HTTP ALPN protocol %q", state.NegotiatedProtocol)
	}

	old := config.Clone()
	old.MinVersion = tls.VersionTLS12
	old.MaxVersion = tls.VersionTLS12
	if conn, err := tls.Dial("tcp", proxy.Addr().String(), old); err == nil {
		_ = conn.Close()
		t.Fatal("TLS 1.2 handshake unexpectedly succeeded")
	}
}

func TestMutualTLSIsOptionalAndEnforced(t *testing.T) {
	ca := newIdentity(t, nil, "test-ca", false, false)
	serverID := newIdentity(t, &ca, "localhost", true, false)
	clientID := newIdentity(t, &ca, "test-client", false, true)
	certPath, keyPath := writeIdentity(t, serverID)
	clientCert, err := tls.X509KeyPair(clientID.certPEM, clientID.keyPEM)
	if err != nil {
		t.Fatal(err)
	}
	caPath := filepath.Join(t.TempDir(), "ca.pem")
	if err := os.WriteFile(caPath, ca.certPEM, 0o600); err != nil {
		t.Fatal(err)
	}
	proxy := startProxy(t, Config{
		ListenAddr:       "127.0.0.1:0",
		BackendAddr:      startBackend(t),
		CertFile:         certPath,
		KeyFile:          keyPath,
		ClientCAFile:     caPath,
		MaxConns:         8,
		HandshakeTimeout: 2 * time.Second,
		DialTimeout:      2 * time.Second,
	})
	base := &tls.Config{
		MinVersion: tls.VersionTLS13,
		MaxVersion: tls.VersionTLS13,
		RootCAs:    ca.pool,
		ServerName: "localhost",
	}
	if conn, err := tls.Dial("tcp", proxy.Addr().String(), base); err == nil {
		// TLS 1.3 may report the server's missing-client-certificate alert on
		// the first application read rather than during Dial.
		_, _ = conn.Write([]byte("PING\r\n"))
		_, readErr := conn.Read(make([]byte, 1))
		_ = conn.Close()
		if readErr == nil {
			t.Fatal("client without a certificate unexpectedly connected")
		}
	}
	withClient := base.Clone()
	withClient.Certificates = []tls.Certificate{clientCert}
	_, reply := echoOnce(t, proxy.Addr().String(), withClient)
	if string(reply) != "PING\r\n" {
		t.Fatalf("unexpected reply %q", reply)
	}
}

func TestConfigurationValidation(t *testing.T) {
	serverID := newIdentity(t, nil, "localhost", true, false)
	certPath, keyPath := writeIdentity(t, serverID)
	base := Config{
		ListenAddr:       "127.0.0.1:0",
		BackendAddr:      "127.0.0.1:1",
		CertFile:         certPath,
		KeyFile:          keyPath,
		MaxConns:         1,
		HandshakeTimeout: time.Second,
		DialTimeout:      time.Second,
	}
	for name, mutate := range map[string]func(*Config){
		"missing certificate": func(c *Config) { c.CertFile = "" },
		"missing key":         func(c *Config) { c.KeyFile = "" },
		"zero connections":    func(c *Config) { c.MaxConns = 0 },
		"zero handshake":      func(c *Config) { c.HandshakeTimeout = 0 },
		"zero dial":           func(c *Config) { c.DialTimeout = 0 },
	} {
		t.Run(name, func(t *testing.T) {
			config := base
			mutate(&config)
			if _, err := New(config); err == nil {
				t.Fatal("expected configuration error")
			}
		})
	}
}
