//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.usernames;

import org.signal.libsignal.usernames.Username;

import java.util.List;

import junit.framework.TestCase;
import org.signal.libsignal.protocol.util.Hex;

public class UsernamesTest extends TestCase {

    public void testUsernameGeneration() throws BaseUsernameException {
        String nickname = "SiGNAl";
        List<String> usernames = Username.generateCandidates(nickname, 3, 32);
        assertFalse("Non-zero number of usernames expected", usernames.size() == 0);
        for (String name : usernames) {
            assertTrue(String.format("%s does not start with %s", name, nickname), name.startsWith(nickname));
        }
    }

    public void testInvalidNicknameValidation() throws BaseUsernameException {
        List<String> invalidNicknames = List.of("hi", "way_too_long_to_be_a_reasonable_nickname", "I⍰Unicode", "s p a c e s", "0zerostart");
        for (String nickname : invalidNicknames) {
            try {
                Username.generateCandidates(nickname, 3, 32);
                fail(String.format("'%s' should not be considered valid", nickname));
            } catch (BaseUsernameException ex) {
                // this is fine
            }

        }
    }

    public void testValidUsernameHashing() throws BaseUsernameException {
        String username = "he110.42";
        byte[] hash = Username.hash(username);
        assertEquals(32, hash.length);
        assertEquals("f63f0521eb3adfe1d936f4b626b89558483507fbdb838fc554af059111cf322e", Hex.toStringCondensed(hash));
    }

    public void testToTheProofAndBack() throws BaseUsernameException {
        String username = "hello_signal.42";
        byte[] hash = Username.hash(username);
        assertNotNull(hash);
        byte[] randomness = new byte[32];
        byte[] proof = Username.generateProof(username, randomness);
        assertNotNull(proof);
        assertEquals(128, proof.length);
        Username.verifyProof(proof, hash);
    }

    public void testInvalidHash() throws BaseUsernameException {
        String username = "hello_signal.42";
        byte[] hash = Username.hash(username);
        byte[] proof = Username.generateProof(username, new byte[32]);
        hash[0] = 0;

        try {
            Username.verifyProof(proof, hash);
        } catch (BaseUsernameException ex) {
            assertTrue(ex.getMessage().contains("Username could not be verified"));
        }
    }

    public void testInvalidRandomness() throws BaseUsernameException {
        try {
            Username.generateProof("valid_name.01", new byte[31]);
        } catch (Error err) {
            assertTrue(err.getMessage().contains("Failed to create proof"));
        }
    }

    public void testInvalidUsernames() throws BaseUsernameException {
        List<String> usernames = List.of("0zerostart.01", "zero.00", "short_zero.0", "short_one.1");
        for (String name : usernames) {
            try {
                Username.hash(name);
                fail(String.format("'%s' should not be valid", name));
            } catch (BaseUsernameException ex) {
                // this is fine
            }
        }
        for (String name : usernames) {
            try {
                Username.generateProof(name, new byte[32]);
                fail(String.format("'%s' should not be valid", name));
            } catch (BaseUsernameException ex) {
                // this is fine
            }
        }
    }

}
