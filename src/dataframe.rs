//! Module containing the default implementation of data frames.
use std::io::{self, Read, Write};
use result::{WebSocketResult, WebSocketError};
use ws::dataframe::DataFrame as DataFrameable;
use ws::util::header::DataFrameHeader;
use ws::util::header::{self as dfh, DataFrameFlags, ReaderState};
use ws::util::mask;
use uuid::Uuid;
use ::receiver::PacketState;

/// Represents a WebSocket data frame.
///
/// The data held in a DataFrame is never masked.
/// Masking/unmasking is done when sending and receiving the data frame,
///
/// This DataFrame, unlike the standard Message implementation (which also
/// implements the DataFrame trait), owns its entire payload. This means that calls to `payload`
/// don't allocate extra memory (again unlike the default Message implementation).
#[derive(Debug, Clone, PartialEq)]
pub struct DataFrame {
	/// Whether or no this constitutes the end of a message
	pub finished: bool,
	/// The reserved portion of the data frame (RFC6455 5.2)
	pub reserved: [bool; 3],
	/// The opcode associated with this data frame
	pub opcode: Opcode,
	/// The payload associated with this data frame
	pub data: Vec<u8>,
}

impl DataFrame {
	/// Creates a new DataFrame.
	pub fn new(finished: bool, opcode: Opcode, data: Vec<u8>) -> DataFrame {
		DataFrame {
			finished: finished,
			reserved: [false; 3],
			opcode: opcode,
			data: data,
		}
	}

	/// Take the body and header of a dataframe and combine it into a single
	/// Dataframe struct. A websocket message can be made up of many individual
	/// dataframes, use the methods from the Message or OwnedMessage structs to
	/// take many of these and create a websocket message.
	pub fn read_dataframe_body(
		header: DataFrameHeader,
		body: Vec<u8>,
		should_be_masked: bool,
	) -> WebSocketResult<Self> {
		let finished = header.flags.contains(DataFrameFlags::FIN);

		let reserved = [
			header.flags.contains(DataFrameFlags::RSV1),
			header.flags.contains(DataFrameFlags::RSV2),
			header.flags.contains(DataFrameFlags::RSV3),
		];

		let opcode = Opcode::new(header.opcode).expect("Invalid header opcode!");

		let data = match header.mask {
			Some(mask) => {
				if !should_be_masked {
					return Err(WebSocketError::DataFrameError("Expected unmasked data frame"));
				}
				mask::mask_data(mask, &body)
			}
			None => {
				if should_be_masked {
					return Err(WebSocketError::DataFrameError("Expected masked data frame"));
				}
				body
			}
		};

		Ok(DataFrame {
		       finished: finished,
		       reserved: reserved,
		       opcode: opcode,
		       data: data,
		   })
	}

	/// Reads a DataFrame from a Reader.
	pub fn read_dataframe<R>(
		reader: &mut R,
		should_be_masked: bool,
		uuid: Uuid,
		packet_state: &mut PacketState,
		reader_state: &mut ReaderState,
	) -> WebSocketResult<Self>
		where R: Read
	{
		trace!("Reading data for {}", uuid);

		let frame = {
			//	If a header was read previously, use that. Otherwise read a new one.
			if packet_state.header.is_none() {
				packet_state.header = Some(dfh::read_header(reader, reader_state)?);
			}

			let header = packet_state.header.unwrap();

			//	If this is a new packet, allocate space for it.
			if packet_state.packet.capacity() != header.len as usize {
				packet_state.packet = Vec::with_capacity(header.len as usize);
			}

			let len = packet_state.packet.len();
			let mut data = Vec::new();

			if let Err(why) = reader.take(header.len - len as u64).read_to_end(&mut data) {
				// Could not read entire packet at once
				// Store what we got and return the error.
				debug!("Read failure, read {} bytes", data.len());
				packet_state.packet.append(&mut data);
				debug!("Current packet size: {} / {}", packet_state.packet.len(), header.len);
				return Err(WebSocketError::IoError(why));
			};

			//	Append the last of the data to the packet.
			packet_state.packet.append(&mut data);

			//	If there's still not enough data, then something is wrong.
			if (packet_state.packet.len() as u64) < header.len {
				debug!("Incomplete packet: {} / {}", packet_state.packet.len(), header.len);
				return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "incomplete payload").into());
			}

			match String::from_utf8(packet_state.packet.clone()) {
				Ok(payload) => trace!("WSData: {:?} - {}", packet_state.header, payload),
				Err(_) => trace!("WS Data: {:?}", packet_state),
			}

			DataFrame::read_dataframe_body(header, packet_state.packet.clone(), should_be_masked)
		};

		// This is the end, so reset the header and packet states.
		packet_state.reset();

		frame
	}
}

impl DataFrameable for DataFrame {
	#[inline(always)]
	fn is_last(&self) -> bool {
		self.finished
	}

	#[inline(always)]
	fn opcode(&self) -> u8 {
		self.opcode as u8
	}

	#[inline(always)]
	fn reserved(&self) -> &[bool; 3] {
		&self.reserved
	}

	#[inline(always)]
	fn size(&self) -> usize {
		self.data.len()
	}

	#[inline(always)]
	fn write_payload(&self, socket: &mut Write) -> WebSocketResult<()> {
		socket.write_all(self.data.as_slice())?;
		Ok(())
	}

	#[inline(always)]
	fn take_payload(self) -> Vec<u8> {
		self.data
	}
}

/// Represents a WebSocket data frame opcode
#[derive(Clone, Debug, Copy, PartialEq)]
pub enum Opcode {
	/// A continuation data frame
	Continuation,
	/// A UTF-8 text data frame
	Text,
	/// A binary data frame
	Binary,
	/// An undefined non-control data frame
	NonControl1,
	/// An undefined non-control data frame
	NonControl2,
	/// An undefined non-control data frame
	NonControl3,
	/// An undefined non-control data frame
	NonControl4,
	/// An undefined non-control data frame
	NonControl5,
	/// A close data frame
	Close,
	/// A ping data frame
	Ping,
	/// A pong data frame
	Pong,
	/// An undefined control data frame
	Control1,
	/// An undefined control data frame
	Control2,
	/// An undefined control data frame
	Control3,
	/// An undefined control data frame
	Control4,
	/// An undefined control data frame
	Control5,
}

impl Opcode {
	/// Attempts to form an Opcode from a nibble.
	///
	/// Returns the Opcode, or None if the opcode is out of range.
	pub fn new(op: u8) -> Option<Opcode> {
		Some(match op {
		         0 => Opcode::Continuation,
		         1 => Opcode::Text,
		         2 => Opcode::Binary,
		         3 => Opcode::NonControl1,
		         4 => Opcode::NonControl2,
		         5 => Opcode::NonControl3,
		         6 => Opcode::NonControl4,
		         7 => Opcode::NonControl5,
		         8 => Opcode::Close,
		         9 => Opcode::Ping,
		         10 => Opcode::Pong,
		         11 => Opcode::Control1,
		         12 => Opcode::Control2,
		         13 => Opcode::Control3,
		         14 => Opcode::Control4,
		         15 => Opcode::Control5,
		         _ => return None,
		     })
	}
}

#[cfg(all(feature = "nightly", test))]
mod tests {
	use super::*;
	use ws::dataframe::DataFrame as DataFrameable;
	use test::Bencher;

	#[test]
	fn test_read_dataframe() {
		let data = b"The quick brown fox jumps over the lazy dog";
		let mut dataframe = vec![0x81, 0x2B];
		for i in data.iter() {
			dataframe.push(*i);
		}
		let obtained = DataFrame::read_dataframe(&mut &dataframe[..], false).unwrap();
		let expected = DataFrame {
			finished: true,
			reserved: [false; 3],
			opcode: Opcode::Text,
			data: data.to_vec(),
		};
		assert_eq!(obtained, expected);
	}

	#[test]
	fn read_incomplete_payloads() {
		let mut data = vec![0x8au8, 0x08, 0x19, 0xac, 0xab, 0x8a, 0x52, 0x4e, 0x05, 0x00];
		let payload = vec![25, 172, 171, 138, 82, 78, 5, 0];
		let short_header = DataFrame::read_dataframe(&mut &data[..1], false);
		let short_payload = DataFrame::read_dataframe(&mut &data[..6], false);
		let full_payload = DataFrame::read_dataframe(&mut &data[..], false);
		data.push(0xff);
		let more_payload = DataFrame::read_dataframe(&mut &data[..], false);

		match (short_header.unwrap_err(), short_payload.unwrap_err()) {
			(WebSocketError::NoDataAvailable, WebSocketError::NoDataAvailable) => (),
			_ => assert!(false),
		};
		assert_eq!(full_payload.unwrap().data, payload);
		assert_eq!(more_payload.unwrap().data, payload);
	}

	#[bench]
	fn bench_read_dataframe(b: &mut Bencher) {
		let data = b"The quick brown fox jumps over the lazy dog";
		let mut dataframe = vec![0x81, 0x2B];
		for i in data.iter() {
			dataframe.push(*i);
		}
		b.iter(|| { DataFrame::read_dataframe(&mut &dataframe[..], false).unwrap(); });
	}

	#[test]
	fn test_write_dataframe() {
		let data = b"The quick brown fox jumps over the lazy dog";
		let mut expected = vec![0x81, 0x2B];
		for i in data.iter() {
			expected.push(*i);
		}
		let dataframe = DataFrame {
			finished: true,
			reserved: [false; 3],
			opcode: Opcode::Text,
			data: data.to_vec(),
		};
		let mut obtained = Vec::new();
		dataframe.write_to(&mut obtained, false).unwrap();

		assert_eq!(&obtained[..], &expected[..]);
	}

	#[bench]
	fn bench_write_dataframe(b: &mut Bencher) {
		let data = b"The quick brown fox jumps over the lazy dog";
		let dataframe = DataFrame {
			finished: true,
			reserved: [false; 3],
			opcode: Opcode::Text,
			data: data.to_vec(),
		};
		let mut writer = Vec::with_capacity(45);
		b.iter(|| { dataframe.write_to(&mut writer, false).unwrap(); });
	}
}
