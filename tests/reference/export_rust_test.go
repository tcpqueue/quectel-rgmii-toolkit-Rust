package main

import (
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"testing"
	"time"
)

var rustSMSInputs []string
var rustCaptureSMS bool

func captureRustSMS(raw string) {
	if rustCaptureSMS {
		rustSMSInputs = append(rustSMSInputs, raw)
	}
}

func TestExportRustFixtures(t *testing.T) {
	rustCaptureSMS = true
	for _, test := range []func(*testing.T){TestParseSMSListMergesSameSenderSameDateFragments, TestParseSMSListMergesSameSenderWithinFiveSeconds, TestParseSMSListKeepsContinuousIndexOrder, TestParseSMSListKeepsNumberedMenuOrder, TestParseSMSListKeepsPointsExchangeFragmentsInStorageOrder, TestParseSMSListPDUModeUCS2KeepsNewlines, TestParseSMSListPDUModeUCS2ConvertsCarriageReturnToNewline, TestParseSMSListPDUModeAddsTextLines} {
		test(t)
	}
	rustCaptureSMS = false
	inputs := map[string]bool{"": true, defaultMockDashboardATResponse(""): true}
	files, _ := filepath.Glob("*.go")
	for _, path := range files {
		if path == "export_rust_test.go" {
			continue
		}
		tree, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if err != nil {
			t.Fatal(err)
		}
		ast.Inspect(tree, func(n ast.Node) bool {
			literal, ok := n.(*ast.BasicLit)
			if !ok || literal.Kind != token.STRING {
				return true
			}
			value, err := strconv.Unquote(literal.Value)
			if err == nil && len(value) < 16000 && strings.ContainsAny(value, "\r\n") && (strings.Contains(value, "+Q") || strings.Contains(value, "+CG") || strings.Contains(value, "+CMGL:")) {
				inputs[value] = true
			}
			return true
		})
	}
	raws := make([]string, 0, len(inputs))
	for raw := range inputs {
		raws = append(raws, raw)
	}
	sort.Strings(raws)
	fixtures := []map[string]any{}
	parsers := map[string]func(string) map[string]any{"dashboard": parseDashboardAT, "device": parseDeviceInfoAT, "network": parseNetworkSettingsAT, "bands": parseNetworkBandsAT, "settings": parseSettingsStatusAT, "scan": parseCellScanAT}
	for _, raw := range raws {
		for _, kind := range []string{"dashboard", "device", "network", "bands", "settings", "scan"} {
			fixtures = append(fixtures, map[string]any{"kind": kind, "raw": raw, "expected": parsers[kind](raw)})
		}
	}
	data, err := json.MarshalIndent(fixtures, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(os.Getenv("RUST_FIXTURE_ROOT"), "tests/fixtures/go-parsers.json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	mocks := map[string]string{}
	for _, key := range []string{atKeyDashboard, atKeyDeviceInfo, atKeyNetworkBands, atKeyNetworkSettings, atKeySettingsStatus, atKeyModel, atKeySIMStatus, atKeySMSList, atKeyIMSI} {
		for _, cmd := range pageATCommands(key) {
			mocks[cmd] = mockATResponse(cmd)
		}
	}
	data, err = json.MarshalIndent(mocks, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(os.Getenv("RUST_FIXTURE_ROOT"), "tests/fixtures/mock-at.json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	t.Logf("exported %d parser cases", len(fixtures))
	sms := []map[string]any{}
	for _, raw := range rustSMSInputs {
		sms = append(sms, map[string]any{"raw": raw, "expected": parseSMSListAT(raw)})
	}
	for _, raw := range raws {
		if strings.Contains(raw, "%d") || strings.Contains(raw, "%s") {
			continue
		}
		sms = append(sms, map[string]any{"raw": raw, "expected": parseSMSListAT(raw)})
	}
	data, _ = json.MarshalIndent(sms, "", "  ")
	os.WriteFile(filepath.Join(os.Getenv("RUST_FIXTURE_ROOT"), "tests/fixtures/go-sms.json"), data, 0644)
	codes := map[string]string{}
	for i := 100; i < 1000; i++ {
		mcc := strconv.Itoa(i)
		if code := mccToCallingCode(mcc); code != "" {
			codes[mcc] = code
		}
	}
	data, _ = json.Marshal(codes)
	os.WriteFile(filepath.Join(os.Getenv("RUST_FIXTURE_ROOT"), "src/calling-codes.json"), data, 0644)
	os.WriteFile(filepath.Join(os.Getenv("RUST_FIXTURE_ROOT"), "src/console.html"), []byte(nativeConsoleHTML), 0644)
	atCases := []map[string]any{}
	for _, path := range files {
		tree, _ := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if tree == nil {
			continue
		}
		ast.Inspect(tree, func(n ast.Node) bool {
			lit, ok := n.(*ast.BasicLit)
			if !ok || lit.Kind != token.STRING {
				return true
			}
			value, err := strconv.Unquote(lit.Value)
			if err == nil && strings.HasPrefix(value, "AT") && !strings.ContainsAny(value, "\r\n") && len(value) < 4096 {
				atCases = append(atCases, map[string]any{"command": value, "action": isATActionCommand(value), "timeout": atCommandTimeoutMS(value), "maxAge": maxAgeForATCacheCommand(value).Milliseconds()})
			}
			return true
		})
	}
	data, _ = json.MarshalIndent(atCases, "", "  ")
	os.WriteFile(filepath.Join(os.Getenv("RUST_FIXTURE_ROOT"), "tests/fixtures/go-at-policy.json"), data, 0644)
	actionCases := []map[string]any{}
	for _, entry := range []struct{ page, query string }{
		{"device", "action=set_imei&imei=123456789012345"},
		{"network", "action=lock_bands&mode=LTE&values=1:3:7"}, {"network", "action=lock_bands&mode=NSA&values=78:79"}, {"network", "action=lock_bands&mode=SA&values=28:78"},
		{"network", "action=reset_bands&lte=1:3&nsa=41:78&sa=28:78"}, {"network", "action=unlock_lte"}, {"network", "action=unlock_nr"},
		{"network", "action=lock_nr_manual&pci=0&earfcn=633984&scs=30&band=78"},
		{"network", "action=lock_lte_manual&cellNum=2&pairs=1850,1%3B1650,0"},
		{"network", "action=lock_scanned_cells&mode=NR5G+Only&pci=5&earfcn=633984&band=78"},
		{"network", "action=lock_scanned_cells&mode=LTE+Only&earfcn=1850,1650&pci=0,2"},
		{"network", "action=save_settings&apn=cmnet&pdpType=IPV4V6&modePref=AUTO&nrDisableMode=0"},
		{"network", "action=save_settings&modePref=LTE:NR5G&nrDisableMode=2"},
		{"settings", "action=manual_at&command=ATI"}, {"settings", "action=set_imei&imei=123456789012345"}, {"settings", "action=reboot"}, {"settings", "action=reset_at"},
		{"settings", "action=ip_passthrough&mode=ETH&enabled=true"}, {"settings", "action=ip_passthrough&mode=USB&enabled=true"},
		{"settings", "action=dns_proxy&family=4&enabled=true"}, {"settings", "action=dns_proxy&family=6&enabled=false"},
		{"settings", "action=usbnet&mode=ECM"}, {"settings", "action=usbnet&mode=RMNET"}, {"settings", "action=usbnet&mode=MBIM"}, {"settings", "action=usbnet&mode=RNDIS"},
		{"settings", "action=dmz&enabled=true&ip=192.168.225.10"}, {"settings", "action=dmz&enabled=false"},
		{"settings", "action=lanip&start=192.168.225.2&end=192.168.225.100&gateway=192.168.225.1"},
	} {
		atCommandCache = &atCommandCacheManager{entries: make(map[string]*atCacheEntry), queue: make(chan string, 64), readyAt: time.Now()}
		app := &simpleAdminServer{cfg: serverConfig{mockMode: true}}
		req := httptest.NewRequest("GET", "/api/test?"+entry.query, nil)
		rr := httptest.NewRecorder()
		switch entry.page {
		case "device":
			app.handleDeviceInfoData(rr, req)
		case "network":
			app.handleNetworkData(rr, req)
		case "settings":
			app.handleSettingsData(rr, req)
		}
		atCommandCache.mu.Lock()
		cmds := []string{}
		for cmd := range atCommandCache.entries {
			cmds = append(cmds, cmd)
		}
		atCommandCache.mu.Unlock()
		sort.Strings(cmds)
		actionCases = append(actionCases, map[string]any{"page": entry.page, "query": entry.query, "commands": cmds, "status": rr.Code})
	}
	data, _ = json.MarshalIndent(actionCases, "", "  ")
	os.WriteFile(filepath.Join(os.Getenv("RUST_FIXTURE_ROOT"), "tests/fixtures/go-actions.json"), data, 0644)
}
