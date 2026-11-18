/// A key made for the UI tests alone (ssh-keygen, ed25519, under ``passphrase``), never a machine's.
internal enum TestKey {
    /// The passphrase the key is under.
    internal static let passphrase = "qa-pass-mk7r"
    /// The private key, in OpenSSH's format.
    internal static let privateKey = """
        -----BEGIN OPENSSH PRIVATE KEY-----
        b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABCWngIYVP
        VfoDphEkgjvf91AAAAIAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIAQf4nqcnoLFMdd2
        Oiv6z8HrIpFslGrrx4lZRZnMxyJbAAAAkNAV6oDZnuBaC4bgT40ab7mXuhc3SFz+az1Yhh
        I+o77Iyh5R6zc8eqAt77knhwXoO4xwYTrz9Kksmzjq/FL34xgQGcxBWPjJwiAGgYZpHpTe
        G3YEmpNoPCh4DIdIPAi0iglvnx53LueqWq6bk8b4WnC3LeyHiiq+ruDes7AXg8Jwrh3Mar
        j+YSTabiXDgS4ZmQ==
        -----END OPENSSH PRIVATE KEY-----
        """
}
