import Testing

@testable import MacDaemon

// Fixtures are verbatim `networksetup` output captured on macOS 26.5.

@Suite("parseServiceList")
struct ParseServiceListTests {
    @Test("drops the header line and returns every service")
    func realOutput() {
        let output = """
            An asterisk (*) denotes that a network service is disabled.
            USB 10/100/1000 LAN
            Thunderbolt Bridge
            Wi-Fi
            Tailscale
            """

        let services = NetworkServices.parseServiceList(output)

        #expect(
            services == [
                NetworkService(name: "USB 10/100/1000 LAN", isEnabled: true),
                NetworkService(name: "Thunderbolt Bridge", isEnabled: true),
                NetworkService(name: "Wi-Fi", isEnabled: true),
                NetworkService(name: "Tailscale", isEnabled: true),
            ])
    }

    @Test("strips the asterisk from disabled services and records them as disabled")
    func disabledServices() {
        let output = """
            An asterisk (*) denotes that a network service is disabled.
            *Thunderbolt Bridge
            Wi-Fi
            """

        let services = NetworkServices.parseServiceList(output)

        // The asterisk must not survive into the name — networksetup rejects "*Thunderbolt
        // Bridge" as an unknown service, so a leaked prefix silently breaks configuration.
        #expect(services == [
            NetworkService(name: "Thunderbolt Bridge", isEnabled: false),
            NetworkService(name: "Wi-Fi", isEnabled: true),
        ])
    }

    @Test("preserves internal spaces in service names")
    func namesWithSpaces() {
        let output = """
            An asterisk (*) denotes that a network service is disabled.
            USB 10/100/1000 LAN
            """

        #expect(NetworkServices.parseServiceList(output).first?.name == "USB 10/100/1000 LAN")
    }

    @Test("handles a header-only list, empty input, and blank lines")
    func degenerateInput() {
        #expect(
            NetworkServices.parseServiceList(
                "An asterisk (*) denotes that a network service is disabled.").isEmpty)
        #expect(NetworkServices.parseServiceList("").isEmpty)
        #expect(NetworkServices.parseServiceList("\n\n").isEmpty)
    }

    @Test("tolerates missing header and trailing whitespace")
    func noHeader() {
        let services = NetworkServices.parseServiceList("Wi-Fi   \n  \nEthernet\n")
        #expect(services == [
            NetworkService(name: "Wi-Fi", isEnabled: true),
            NetworkService(name: "Ethernet", isEnabled: true),
        ])
    }
}

@Suite("parseProxySetting")
struct ParseProxySettingTests {
    @Test("parses a disabled proxy with an empty server")
    func disabled() {
        let output = """
            Enabled: No
            Server:\u{20}
            Port: 0
            Authenticated Proxy Enabled: 0
            """

        #expect(
            NetworkServices.parseProxySetting(output)
                == ProxySetting(isEnabled: false, server: "", port: 0))
    }

    @Test("parses an enabled proxy")
    func enabled() {
        let output = """
            Enabled: Yes
            Server: 127.0.0.1
            Port: 8080
            Authenticated Proxy Enabled: 0
            """

        #expect(
            NetworkServices.parseProxySetting(output)
                == ProxySetting(isEnabled: true, server: "127.0.0.1", port: 8080))
    }

    @Test("returns nil when the output is not a proxy report")
    func malformed() {
        #expect(NetworkServices.parseProxySetting("") == nil)
        #expect(
            NetworkServices.parseProxySetting(
                "** Error: The parameters were not valid.") == nil)
    }

    @Test("treats an unparseable port as absent rather than defaulting to a live port")
    func badPort() {
        let output = """
            Enabled: Yes
            Server: proxy.example.com
            Port: not-a-number
            """

        #expect(NetworkServices.parseProxySetting(output) == nil)
    }
}

@Suite("parseBypassDomains")
struct ParseBypassDomainsTests {
    @Test("parses a populated bypass list")
    func populated() {
        #expect(
            NetworkServices.parseBypassDomains("*.local\n169.254/16\n")
                == ["*.local", "169.254/16"])
    }

    @Test("returns an empty list for the no-domains message")
    func empty() {
        // networksetup reports absence in prose rather than with an empty body; treating that
        // sentence as a domain would write it back as a literal bypass rule on restore.
        #expect(
            NetworkServices.parseBypassDomains(
                "There aren't any bypass domains set on this network service.").isEmpty)
        #expect(NetworkServices.parseBypassDomains("").isEmpty)
    }
}
