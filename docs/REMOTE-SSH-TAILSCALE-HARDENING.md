# Mac Remote SSH + Tailscale Hardening Plan

> Goal: Let an iPhone connect to this Mac over SSH through the Tailscale private network and continue using the local `Codex CLI`, subject to these constraints:
>
> - Only the specified source device may access port `22`.
> - Only SSH key authentication is allowed.
> - Remote SSH login with an account password is prohibited.
> - Local login, unlocking, and `sudo` at the computer must still accept the account password.

---

## Prerequisites

This plan assumes:

- **Ordinary SSH over Tailscale**
- The built-in macOS **Remote Login** service
- **Tailscale SSH is not enabled**

Reasons:

- This path follows the native terminal workflow: `iPhone -> Tailscale -> SSH -> tmux -> Codex CLI`.
- You retain full control of `sshd_config`, `authorized_keys`, and local SSH behavior.
- If you later switch to `Tailscale SSH`, Tailscale takes over port `22` for tailnet traffic. The control plane moves to Tailscale policy rather than the local `sshd_config`.

---

## Sensitive Information Boundary

All usernames, device names, domains, Tailscale IPs, and SSH example values in this document are placeholders.

- Actual `macOS` usernames
- Actual Tailscale device names and fully qualified domain names
- Actual Tailscale IPv4 addresses
- Actual SSH Host configuration and private key identifiers

Store these values in the secrets repository or an equivalent private configuration repository. Do not commit them to this repository's code, documentation, or examples.

---

## Recorded Machine State

The local inspection of this Mac recorded the following:

- `com.openssh.sshd` was enabled.
- Local `127.0.0.1:22` was reachable.
- The on-disk [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config) already enforced the basic key-only, no-password restrictions.

The key settings in the main configuration were:

```sshconfig
PermitRootLogin no
PubkeyAuthentication yes
AuthenticationMethods publickey
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
AllowUsers YOUR_MAC_USER
```

This means:

- Remote SSH was already **key-only**.
- Remote SSH already **disabled password login**.
- The remaining gaps were mainly:
  - The source device had not yet been restricted to the specified iPhone.
  - Access to port `22` had not yet been explicitly narrowed in Tailscale policy.

---

## Recommended Final Architecture

Use three layers of control. Do not put every security policy into [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config).

### Layer 1: Remote Login

In macOS `General -> Sharing -> Remote Login`:

- Enable `Remote Login`.
- Allow only your own Mac account.
- Do not use `All users`.

This layer determines:

- Which local accounts are eligible for remote login.

### Layer 2: Tailscale Policy

In the tailnet's `Access controls`:

- Allow only `your iPhone -> this Mac:22`.
- Do not allow other tailnet devices to access this Mac's SSH port.

This layer determines:

- Which source devices can reach port `22`.

### Layer 3: OpenSSH (`sshd_config`)

In the local SSH configuration:

- Allow only user `YOUR_MAC_USER`.
- Allow only the specified iPhone's Tailscale IP as the source.
- Allow only `publickey`.
- Prohibit password login.
- Prohibit remote `root` login.

This layer ensures:

- Even if someone reaches the host through the tailnet, the username, source address, and key must all meet the restrictions.

---

## Intended Outcome

The final state should be:

- The iPhone can log in with an `SSH key`.
- The iPhone cannot log in with an account password.
- Other tailnet devices cannot access this Mac's port `22`.
- Local system login, unlocking, and `sudo` at the computer continue to accept the account password.

In other words:

- **Remote SSH authentication** is restricted.
- **Local account passwords** are unaffected.

---

## Collect These 4 Values Before Implementation

Before making changes, confirm:

1. Your Mac username, such as `YOUR_MAC_USER`.
2. This Mac's Tailscale device name, such as `YOUR_MAC_DEVICE_NAME`.
3. This Mac's Tailscale FQDN, such as `YOUR_MAC_TAILNET_FQDN`.
4. Your iPhone's Tailscale IPv4 address, such as `YOUR_IPHONE_TAILSCALE_IP`.

Item 4 is the most critical because it is used to restrict the source address.

---

## Recommended Implementation Sequence

### Step 1: Confirm the Public Key Is Installed

Confirm that the public key corresponding to the iPhone's key is present in `~/.ssh/authorized_keys` in the local user's home directory.

Recommended permissions:

```bash
chmod 700 ~/.ssh
chmod 600 ~/.ssh/authorized_keys
```

If this is not ready, stop before tightening the configuration further.

### Step 2: Back Up the Main SSH Configuration

```bash
sudo cp /etc/ssh/sshd_config /etc/ssh/sshd_config.bak-$(date +%Y%m%d%H%M%S)
```

### Step 3: Choose the Configuration Location

At the time of inspection, this machine already had custom restrictions directly at the bottom of [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config).

There are two approaches:

#### Approach A: Minimal Change for This Machine

Continue using the main file. Change only:

```sshconfig
AllowUsers YOUR_MAC_USER
```

to:

```sshconfig
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

Replace `YOUR_IPHONE_TAILSCALE_IP` with your iPhone's actual Tailscale IP.

#### Approach B: Later Cleanup with a Drop-in File

The inspected machine supported [`/etc/ssh/sshd_config.d`](/etc/ssh/sshd_config.d), and the beginning of the main configuration already contained:

```sshconfig
Include /etc/ssh/sshd_config.d/*
```

To separate local customizations from system defaults later, create:

[`/etc/ssh/sshd_config.d/900-local-hardening.conf`](/etc/ssh/sshd_config.d/900-local-hardening.conf)

with:

```sshconfig
PermitRootLogin no
PubkeyAuthentication yes
AuthenticationMethods publickey
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

Then remove the duplicated custom settings at the bottom of the main file to avoid maintaining them in multiple places.

For the recorded state, **Approach A** is recommended first because this machine already maintained these rules in the main file.

---

## Recommended Final `sshd_config`

Assuming:

- The username is `YOUR_MAC_USER`.
- The iPhone's Tailscale IP is `YOUR_IPHONE_TAILSCALE_IP`.

The recommended final SSH restrictions are:

```sshconfig
PermitRootLogin no
PubkeyAuthentication yes
AuthenticationMethods publickey
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

### Meaning of Each Setting

- `PermitRootLogin no`
  - Prohibits direct remote login as root.
- `PubkeyAuthentication yes`
  - Enables public key authentication.
- `AuthenticationMethods publickey`
  - Requires public key authentication exclusively.
- `PasswordAuthentication no`
  - Prohibits remote password authentication.
- `KbdInteractiveAuthentication no`
  - Prohibits keyboard-interactive password authentication.
- `PermitEmptyPasswords no`
  - Prohibits empty passwords.
- `AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP`
  - Allows only user `YOUR_MAC_USER`, originating from this iPhone's address.

### Configuration to Avoid

Do not broaden the source restriction to the entire Tailscale IPv4 address pool merely for convenience:

```sshconfig
AllowUsers YOUR_MAC_USER@100.64.0.0/10
```

This is technically possible, but the security boundary is too broad. Since the goal is two layers of restriction, use a single device's Tailscale IP.

---

## Tailscale Policy Plan

Source device restrictions belong in Tailscale policy as well as `sshd_config`.

### Recommended Target

Allow only:

- `your iPhone -> your Mac:22`

Do not allow:

- Other tailnet devices to access this Mac's port `22`.

### `grants` Example

If your tailnet policy already uses `grants`, use `hosts` aliases with `grants`:

```json
{
  "hosts": {
    "mac_remote": "YOUR_MAC_TAILSCALE_IP",
    "iphone_remote": "YOUR_IPHONE_TAILSCALE_IP"
  },
  "grants": [
    {
      "src": ["iphone_remote"],
      "dst": ["mac_remote"],
      "ip": ["tcp:22"]
    }
  ]
}
```

Notes:

- Both `src` and `dst` can use aliases defined in `hosts`.
- Only `tcp:22` is allowed in `ip`.
- Prefer `grants` for a new tailnet policy.

### `acls` Example

If your tailnet still uses `acls`, use this as a reference:

```json
{
  "hosts": {
    "mac_remote": "YOUR_MAC_TAILSCALE_IP",
    "iphone_remote": "YOUR_IPHONE_TAILSCALE_IP"
  },
  "acls": [
    {
      "action": "accept",
      "src": ["iphone_remote"],
      "dst": ["mac_remote:22"]
    }
  ]
}
```

Notes:

- Do not replace the entire existing policy.
- Merge this rule into existing business rules, if any.
- The names in `hosts` improve readability; these exact names are not required.

---

## Why Both Layers Are Needed

### Using Only `sshd_config`

With only:

- `AllowUsers YOUR_MAC_USER@100.x.y.z`
- `PasswordAuthentication no`

one issue remains:

- Other tailnet devices can still reach your port `22`.
- Their login attempts merely fail at authentication.

Blocking access in Tailscale closes the entry point earlier.

### Using Only a Tailscale ACL

With only:

- `iPhone -> Mac:22`

and no tighter `sshd_config`, the remote login boundary can weaken if SSH key or user configuration goes wrong, or if password authentication is accidentally enabled later.

### Using Both Layers

- Tailscale determines who can reach the service.
- OpenSSH determines who can authenticate after reaching it.

This provides a clear, reliable separation of controls.

---

## Execution Order

Follow this order strictly to avoid locking yourself out:

1. Confirm that the iPhone's public key is in `~/.ssh/authorized_keys`.
2. Confirm that SSH key login to this Mac currently works.
3. Back up [`/etc/ssh/sshd_config`](/etc/ssh/sshd_config).
4. Tighten `AllowUsers YOUR_MAC_USER` to `AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP`.
5. Validate the configuration syntax.
6. Keep the current session open and start a new SSH test session.
7. Verify that key login succeeds.
8. Verify that password login fails.
9. Tighten access to port `22` in Tailscale `Access controls`.
10. Verify an actual connection using `Termius` on the iPhone.

---

## Verification Commands

### 1. Validate SSH Configuration Syntax

```bash
sudo /usr/sbin/sshd -t
```

No output indicates success.

### 2. Verify Key Login Succeeds

Test from another device or terminal:

```bash
ssh YOUR_MAC_USER@YOUR_MAC_TAILNET_FQDN
```

or:

```bash
ssh YOUR_MAC_USER@YOUR_MAC_TAILSCALE_IP
```

The connection should open a shell successfully.

### 3. Verify Password Login Fails

Disable key authentication explicitly and try only password / keyboard-interactive authentication:

```bash
ssh \
  -o PubkeyAuthentication=no \
  -o PreferredAuthentications=password,keyboard-interactive \
  YOUR_MAC_USER@YOUR_MAC_TAILNET_FQDN
```

Expected result:

- Login must not succeed.
- Authentication should fail rather than open a shell.

### 4. Verify an Unauthorized Source Fails

Test from a tailnet device other than your iPhone:

- If the Tailscale ACL is effective, access should be denied at the network layer.
- Even if the ACL is not yet effective, `AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP` should prevent successful login.

---

## Rollback

If the change locks you out:

1. Return to the local desktop or a local terminal.
2. Restore the backup:

```bash
sudo cp /etc/ssh/sshd_config.bak-YYYYMMDDHHMMSS /etc/ssh/sshd_config
```

3. Validate again:

```bash
sudo /usr/sbin/sshd -t
```

4. Test SSH again.

Until the new session has been verified, **do not close the currently working session**.

---

## Approaches to Keep Separate

Do not mix the following into this plan; doing so complicates the control plane:

- `Tailscale SSH`
- Exposing port `22` to the public internet
- Relying only on the macOS graphical Firewall for source allowlisting
- Allowing the entire `100.64.0.0/10` range in `sshd_config`

---

## Direct Recommendation for This Machine

Given the recorded machine state, the smallest practical change has two parts:

1. Keep the existing key-only restrictions.
2. Change:

```sshconfig
AllowUsers YOUR_MAC_USER
```

to:

```sshconfig
AllowUsers YOUR_MAC_USER@YOUR_IPHONE_TAILSCALE_IP
```

Then add a Tailscale policy that allows only this iPhone to access this Mac's port `22`.

This preserves local account password use while restricting remote SSH to your phone and your key.
