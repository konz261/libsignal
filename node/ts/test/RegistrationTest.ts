//
// Copyright 2025 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

import { assert, config, expect, use } from 'chai';
import * as chaiAsPromised from 'chai-as-promised';
import * as sinonChai from 'sinon-chai';
import * as util from './util';
import * as Native from '../../Native';
import { ErrorCode, LibSignalErrorBase } from '../Errors';
import {
  newNativeHandle,
  RegistrationService,
  RegistrationSessionState,
  TokioAsyncContext,
} from '../net';
import { InternalRequest } from './NetTest';

use(chaiAsPromised);
use(sinonChai);

util.initLogger();
config.truncateThreshold = 0;

describe('Registration types', () => {
  describe('registration session conversion', () => {
    const expectedSession: RegistrationSessionState = {
      allowedToRequestCode: true,
      verified: true,
      nextCallSecs: 123,
      nextSmsSecs: 456,
      nextVerificationAttemptSecs: 789,
      requestedInformation: new Set(['pushChallenge']),
    };

    const convertedSession = RegistrationService._convertNativeSessionState(
      newNativeHandle(Native.TESTING_RegistrationSessionInfoConvert())
    );
    expect(convertedSession).to.deep.equal(expectedSession);
  });

  expect(() =>
    Native.TESTING_RegistrationService_CreateSessionErrorConvert(
      'InvalidSessionId'
    )
  ).throws(LibSignalErrorBase);

  describe('error conversion', () => {
    const retryLaterCase: [string, object] = [
      'RetryAfter42Seconds',
      {
        code: ErrorCode.RateLimitedError,
        retryAfterSecs: 42,
      },
    ];
    const unknownCase: [string, object] = [
      'Unknown',
      {
        code: ErrorCode.Generic,
        message: 'some message',
      },
    ];
    const timeoutCase: [string, ErrorCode] = ['Timeout', ErrorCode.IoError];
    const cases: Array<{
      operationName: string;
      convertFn: (_: string) => void;
      cases: Array<[string, ErrorCode | object]>;
    }> = [
      {
        operationName: 'CreateSession',
        convertFn: Native.TESTING_RegistrationService_CreateSessionErrorConvert,
        cases: [
          ['InvalidSessionId', ErrorCode.Generic],
          retryLaterCase,
          unknownCase,
          timeoutCase,
        ],
      },
      {
        operationName: 'ResumeSession',
        convertFn: Native.TESTING_RegistrationService_ResumeSessionErrorConvert,
        cases: [
          ['InvalidSessionId', ErrorCode.Generic],
          ['SessionNotFound', ErrorCode.Generic],
          unknownCase,
          timeoutCase,
        ],
      },
      {
        operationName: 'UpdateSession',
        convertFn: Native.TESTING_RegistrationService_UpdateSessionErrorConvert,
        cases: [
          ['Rejected', ErrorCode.Generic],
          retryLaterCase,
          unknownCase,
          timeoutCase,
        ],
      },
      {
        operationName: 'RequestVerificationCode',
        convertFn:
          Native.TESTING_RegistrationService_RequestVerificationCodeErrorConvert,
        cases: [
          ['InvalidSessionId', ErrorCode.Generic],
          ['SessionNotFound', ErrorCode.Generic],
          ['NotReadyForVerification', ErrorCode.Generic],
          ['SendFailed', ErrorCode.Generic],
          ['CodeNotDeliverable', ErrorCode.Generic],
          retryLaterCase,
          unknownCase,
          timeoutCase,
        ],
      },
      {
        operationName: 'SubmitVerification',
        convertFn:
          Native.TESTING_RegistrationService_SubmitVerificationErrorConvert,
        cases: [
          ['InvalidSessionId', ErrorCode.Generic],
          ['SessionNotFound', ErrorCode.Generic],
          ['NotReadyForVerification', ErrorCode.Generic],
          retryLaterCase,
          unknownCase,
          timeoutCase,
        ],
      },
    ];

    cases.forEach(({ operationName, convertFn, cases: testCases }) => {
      it(`converts ${operationName} errors`, () => {
        testCases.forEach(([name, expectation]) => {
          expect(convertFn.bind(Native, name))
            .throws(LibSignalErrorBase)
            .to.include(
              expectation instanceof Object
                ? expectation
                : { code: expectation }
            );
        });
      });
    });
  });
});

describe('Registration client', () => {
  describe('with fake chat remote', () => {
    it('can create a new session', async () => {
      const tokio = new TokioAsyncContext(Native.TokioAsyncContext_new());

      const [createSession, server] = RegistrationService.fakeCreateSession(
        tokio,
        { e164: '+18005550123' }
      );

      const fakeRemote = newNativeHandle(
        await Native.TESTING_FakeChatServer_GetNextRemote(tokio, server)
      );
      const firstRequestHandle =
        await Native.TESTING_FakeChatRemoteEnd_ReceiveIncomingRequest(
          tokio,
          fakeRemote
        );
      assert(firstRequestHandle !== null);
      const firstRequest = new InternalRequest(firstRequestHandle);

      expect(firstRequest.verb).to.eq('POST');
      expect(firstRequest.path).to.eq('/v1/verification/session');

      Native.TESTING_FakeChatRemoteEnd_SendServerResponse(
        fakeRemote,
        newNativeHandle(
          Native.TESTING_FakeChatResponse_Create(
            firstRequest.requestId,
            200,
            'OK',
            ['content-type: application/json'],
            Buffer.from(
              JSON.stringify({
                allowedToRequestCode: true,
                verified: false,
                requestedInformation: ['pushChallenge', 'captcha'],
                id: 'fake-session-A',
              })
            )
          )
        )
      );

      const session = await createSession;
      expect(session.sessionId).to.eq('fake-session-A');
      expect(session.sessionState).property('verified').to.eql(false);
      expect(session.sessionState)
        .property('requestedInformation')
        .to.eql(new Set(['pushChallenge', 'captcha']));
    });
  });
});
