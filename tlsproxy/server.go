// Package main implements a minimal TLS 1.3 terminator for Rillcache's RESP
// protocol. It deliberately uses only the Go standard library.
package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"sync"
	"sync/atomic"
	"time"
)

const bufferSize = 32 * 1024

var bufferPool = sync.Pool{
	New: func() any {
		return make([]byte, bufferSize)
	},
}

// Config describes the TLS listener and the plaintext RESP backend.
type Config struct {
	ListenAddr       string
	BackendAddr      string
	CertFile         string
	KeyFile          string
	ClientCAFile     string
	MaxConns         int
	HandshakeTimeout time.Duration
	DialTimeout      time.Duration
	Logger           *log.Logger
}

// Server terminates TLS 1.3 and proxies RESP bytes to one backend.
type Server struct {
	config    Config
	tlsConfig *tls.Config
	logger    *log.Logger

	listener net.Listener
	sem      chan struct{}
	wg       sync.WaitGroup
	conns    sync.Map // net.Conn -> struct{}
	closing  atomic.Bool
	serving  atomic.Bool
}

// New validates configuration and loads the certificate material.
func New(config Config) (*Server, error) {
	if config.ListenAddr == "" {
		return nil, errors.New("listen address is required")
	}
	if config.BackendAddr == "" {
		return nil, errors.New("backend address is required")
	}
	if config.CertFile == "" || config.KeyFile == "" {
		return nil, errors.New("both certificate and private key files are required")
	}
	if config.MaxConns <= 0 {
		return nil, errors.New("max connections must be positive")
	}
	if config.HandshakeTimeout <= 0 {
		return nil, errors.New("handshake timeout must be positive")
	}
	if config.DialTimeout <= 0 {
		return nil, errors.New("dial timeout must be positive")
	}
	certificate, err := tls.LoadX509KeyPair(config.CertFile, config.KeyFile)
	if err != nil {
		return nil, fmt.Errorf("load certificate/key: %w", err)
	}
	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{certificate},
		MinVersion:   tls.VersionTLS13,
		MaxVersion:   tls.VersionTLS13,
		// RESP clients do not use ALPN. No HTTP/2 or HTTP/3 protocol is offered.
		NextProtos: nil,
	}
	if config.ClientCAFile != "" {
		pem, err := os.ReadFile(config.ClientCAFile)
		if err != nil {
			return nil, fmt.Errorf("read client CA: %w", err)
		}
		pool := x509.NewCertPool()
		if !pool.AppendCertsFromPEM(pem) {
			return nil, errors.New("client CA file contains no certificates")
		}
		tlsConfig.ClientCAs = pool
		tlsConfig.ClientAuth = tls.RequireAndVerifyClientCert
	}
	logger := config.Logger
	if logger == nil {
		logger = log.New(os.Stderr, "tlsproxy: ", log.LstdFlags)
	}
	return &Server{
		config:    config,
		tlsConfig: tlsConfig,
		logger:    logger,
		sem:       make(chan struct{}, config.MaxConns),
	}, nil
}

// Start binds the listener and begins accepting connections. The context only
// controls the accept loop; call Shutdown for a drained stop.
func (s *Server) Start(ctx context.Context) error {
	if !s.serving.CompareAndSwap(false, true) {
		return errors.New("server already started")
	}
	listener, err := net.Listen("tcp", s.config.ListenAddr)
	if err != nil {
		s.serving.Store(false)
		return fmt.Errorf("listen %s: %w", s.config.ListenAddr, err)
	}
	s.listener = listener
	s.wg.Add(1)
	go func() {
		defer s.wg.Done()
		s.acceptLoop(ctx)
	}()
	go func() {
		<-ctx.Done()
		_ = listener.Close()
	}()
	return nil
}

// Addr reports the bound listener address, or nil before Start.
func (s *Server) Addr() net.Addr {
	if s.listener == nil {
		return nil
	}
	return s.listener.Addr()
}

func (s *Server) acceptLoop(ctx context.Context) {
	for {
		raw, err := s.listener.Accept()
		if err != nil {
			if s.closing.Load() || errors.Is(err, net.ErrClosed) {
				return
			}
			select {
			case <-ctx.Done():
				return
			default:
			}
			s.logger.Printf("accept: %v", err)
			continue
		}
		if s.closing.Load() {
			_ = raw.Close()
			return
		}
		select {
		case s.sem <- struct{}{}:
		default:
			_ = raw.Close()
			continue
		}
		s.wg.Add(1)
		go func() {
			defer s.wg.Done()
			defer func() { <-s.sem }()
			s.handle(ctx, raw)
		}()
	}
}

func (s *Server) handle(ctx context.Context, raw net.Conn) {
	defer raw.Close()
	remote := raw.RemoteAddr().String()

	_ = raw.SetDeadline(time.Now().Add(s.config.HandshakeTimeout))
	tlsConn := tls.Server(raw, s.tlsConfig)
	handshakeCtx, cancel := context.WithTimeout(ctx, s.config.HandshakeTimeout)
	err := tlsConn.HandshakeContext(handshakeCtx)
	cancel()
	if err != nil {
		return
	}
	state := tlsConn.ConnectionState()
	if state.Version != tls.VersionTLS13 {
		return
	}
	_ = raw.SetDeadline(time.Time{})

	dialCtx, cancel := context.WithTimeout(ctx, s.config.DialTimeout)
	backendRaw, err := (&net.Dialer{}).DialContext(dialCtx, "tcp", s.config.BackendAddr)
	cancel()
	if err != nil {
		s.logger.Printf("backend dial %s: %v", s.config.BackendAddr, err)
		return
	}
	backend, ok := backendRaw.(*net.TCPConn)
	if !ok {
		_ = backendRaw.Close()
		return
	}
	_ = backend.SetNoDelay(true)
	defer backend.Close()

	connKey := any(tlsConn)
	s.conns.Store(connKey, struct{}{})
	defer s.conns.Delete(connKey)

	s.logger.Printf("connection %s negotiated %s %s", remote, tls.VersionName(state.Version), tls.CipherSuiteName(state.CipherSuite))
	s.proxy(tlsConn, backend)
	_ = tlsConn.Close()
}

func (s *Server) proxy(client *tls.Conn, backend *net.TCPConn) {
	var closeOnce sync.Once
	closeBoth := func() {
		closeOnce.Do(func() {
			_ = client.Close()
			_ = backend.Close()
		})
	}

	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		if err := s.copyDirection(backend, client); err != nil {
			closeBoth()
		}
		_ = backend.CloseWrite()
	}()
	go func() {
		defer wg.Done()
		if err := s.copyDirection(client, backend); err != nil {
			closeBoth()
		}
		_ = client.CloseWrite()
	}()
	wg.Wait()
	closeBoth()
}

func (s *Server) copyDirection(dst io.Writer, src io.Reader) error {
	buffer := bufferPool.Get().([]byte)
	defer bufferPool.Put(buffer)
	_, err := io.CopyBuffer(dst, src, buffer)
	return err
}

// Shutdown stops accepting, waits for active proxied connections, and force
// closes anything still open after the context expires.
func (s *Server) Shutdown(ctx context.Context) error {
	s.closing.Store(true)
	if s.listener != nil {
		_ = s.listener.Close()
	}
	done := make(chan struct{})
	go func() {
		s.wg.Wait()
		close(done)
	}()
	select {
	case <-done:
		return nil
	case <-ctx.Done():
		s.conns.Range(func(key, _ any) bool {
			if conn, ok := key.(net.Conn); ok {
				_ = conn.Close()
			}
			return true
		})
		<-done
		return ctx.Err()
	}
}
